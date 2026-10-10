use std::collections::HashSet;

use rusqlite::{OptionalExtension, TransactionBehavior, params};
use sha2::{Digest, Sha256};

use super::suppression::{reactivate_source_records, supersede_other_revisions};
use crate::storage::{
    CommitDisposition, MemoryEligibility, MemoryEnqueueState, MemoryRecordKind, MemoryRepository,
    PendingSourcePart, SourceChunk, SourceId, SourceKind, SourceMaterial, SourcePart,
    SourcePartRole, SourceReceipt, SourceReceiptState, StorageError, unix_time_ms,
};

const MAX_IMPORT_BYTES: usize = 1024 * 1024;
const MAX_SOURCE_PAGE: usize = 32;
pub(super) const MAX_SOURCE_JOBS: i64 = 128;
pub(in crate::storage) const MAX_RECALLED_SOURCES: usize = 32;
const MAX_RECORD_BYTES: usize = 4096;

impl MemoryRepository {
    pub(crate) async fn get_source(
        &self,
        id: SourceId,
    ) -> Result<Option<SourceMaterial>, StorageError> {
        let source = self
            .connection
            .call(move |connection| load_source(connection, id))
            .await?;
        Ok(source)
    }

    pub(crate) async fn import_source(
        &self,
        path_key: String,
        display_path: String,
        text: String,
    ) -> Result<SourceReceipt, StorageError> {
        if text.len() > MAX_IMPORT_BYTES {
            return Err(StorageError::SourceTooLarge(MAX_IMPORT_BYTES));
        }
        if path_key.trim().is_empty() || display_path.trim().is_empty() {
            return Err(StorageError::InvalidInput("import path is empty"));
        }

        let revision_sha256 = sha256(text.as_bytes());
        let created_at_unix_ms = unix_time_ms()?;
        let result = self
            .connection
            .call(move |connection| {
                let transaction =
                    connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let existing: Option<(i64, String)> = transaction
                    .query_row(
                        "SELECT id, state FROM memory_sources
                         WHERE kind = 'import' AND source_key = ?1 AND revision_sha256 = ?2",
                        params![path_key, revision_sha256],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .optional()?;

                if let Some((source_id, state)) = existing {
                    let id = SourceId(source_id);
                    if state == "suppressed" || source_is_suppressed(&transaction, id)? {
                        let receipt = SourceReceipt {
                            source_id: id,
                            revision_sha256,
                            state: SourceReceiptState::SuppressedRevision,
                        };
                        transaction.commit()?;
                        return Ok(receipt);
                    }
                    let reactivated = state == "superseded";
                    if reactivated {
                        supersede_other_revisions(
                            &transaction,
                            "import",
                            &path_key,
                            id,
                            created_at_unix_ms,
                        )?;
                        transaction.execute(
                            "UPDATE memory_sources SET state = 'active'
                             WHERE id = ?1 AND state = 'superseded'",
                            [source_id],
                        )?;
                        reactivate_source_records(&transaction, id)?;
                    }
                    let receipt = SourceReceipt {
                        source_id: id,
                        revision_sha256,
                        state: if reactivated {
                            SourceReceiptState::Reactivated
                        } else {
                            SourceReceiptState::AlreadyCurrent
                        },
                    };
                    transaction.commit()?;
                    return Ok(receipt);
                }

                supersede_other_revisions(
                    &transaction,
                    "import",
                    &path_key,
                    SourceId(-1),
                    created_at_unix_ms,
                )?;
                transaction.execute(
                    "INSERT INTO memory_sources (
                        kind, source_key, revision_sha256, display_source,
                        created_at_unix_ms, state, extraction_state
                     ) VALUES ('import', ?1, ?2, ?3, ?4, 'active', 'not_applicable')",
                    params![path_key, revision_sha256, display_path, created_at_unix_ms],
                )?;
                let source_id = SourceId(transaction.last_insert_rowid());
                transaction.execute(
                    "INSERT INTO source_parts (source_id, part_index, role, content)
                     VALUES (?1, 0, 'imported', ?2)",
                    params![source_id.0, text],
                )?;
                transaction.commit()?;

                Ok(SourceReceipt {
                    source_id,
                    revision_sha256,
                    state: SourceReceiptState::Created,
                })
            })
            .await?;
        Ok(result)
    }

    /// Returns unchunked source parts in bounded pages; each completed part is
    /// removed from this page query after its tokenizer chunks are committed.
    pub(crate) async fn pending_source_parts(
        &self,
        limit: usize,
    ) -> Result<Vec<PendingSourcePart>, StorageError> {
        let limit = limit.clamp(1, MAX_SOURCE_PAGE) as i64;
        let result = self
            .connection
            .call(move |connection| {
                let mut statement = connection.prepare(
                    "SELECT sp.source_id, sp.part_index
                     FROM source_parts sp
                     JOIN memory_sources s ON s.id = sp.source_id
                     WHERE sp.projection_state = 'pending' AND s.state = 'active'
                       AND NOT EXISTS (
                           SELECT 1 FROM source_suppressions ss WHERE ss.source_id = s.id
                       )
                     ORDER BY sp.source_id, sp.part_index LIMIT ?1",
                )?;
                let keys = statement
                    .query_map([limit], |row| {
                        Ok((SourceId(row.get(0)?), row.get::<_, u32>(1)?))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                let mut pending = Vec::new();
                for (source_id, part_index) in keys {
                    let Some(source) = load_source(connection, source_id)? else {
                        continue;
                    };
                    if let Some(part) = source
                        .parts
                        .iter()
                        .find(|part| part.index == part_index)
                        .cloned()
                    {
                        pending.push(PendingSourcePart { source, part });
                    }
                }
                Ok(pending)
            })
            .await?;
        Ok(result)
    }

    /// Saves tokenizer chunks as attributed source records. Range identity
    /// makes repeating a page safe after a process restart.
    pub(crate) async fn commit_source_chunks(
        &self,
        source_id: SourceId,
        revision_sha256: String,
        part_index: u32,
        chunks: Vec<SourceChunk>,
        part_complete: bool,
    ) -> Result<CommitDisposition, StorageError> {
        if chunks.iter().any(|chunk| {
            chunk.text.is_empty()
                || chunk.text.len() > MAX_RECORD_BYTES
                || chunk.start_byte >= chunk.end_byte
                || chunk.text.len() != (chunk.end_byte - chunk.start_byte) as usize
        }) {
            return Err(StorageError::InvalidInput("invalid source chunk"));
        }

        let result = self
            .connection
            .call(move |connection| {
                let transaction =
                    connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                if !source_revision_is_active(&transaction, source_id, &revision_sha256)? {
                    transaction.commit()?;
                    return Ok(CommitDisposition::SourceChanged);
                }
                let part = load_source_part(&transaction, source_id, part_index)?;
                let Some((role, text)) = part else {
                    transaction.commit()?;
                    return Ok(CommitDisposition::InvalidEvidence);
                };
                for chunk in chunks {
                    let Some(slice) = text.get(chunk.start_byte as usize..chunk.end_byte as usize)
                    else {
                        return Ok(CommitDisposition::InvalidEvidence);
                    };
                    if slice != chunk.text {
                        return Ok(CommitDisposition::InvalidEvidence);
                    }
                    insert_source_chunk(
                        &transaction,
                        source_id,
                        &revision_sha256,
                        part_index,
                        role,
                        chunk,
                    )?;
                }
                if part_complete {
                    transaction.execute(
                        "UPDATE source_parts SET projection_state = 'complete'
                         WHERE source_id = ?1 AND part_index = ?2",
                        params![source_id.0, part_index],
                    )?;
                }
                transaction.commit()?;
                Ok(CommitDisposition::Committed)
            })
            .await?;
        Ok(result)
    }
}
pub(in crate::storage) fn insert_conversation_source(
    transaction: &rusqlite::Transaction<'_>,
    turn_id: i64,
    user_text: &str,
    assistant_text: &str,
    created_at_unix_ms: i64,
    eligibility: MemoryEligibility,
    recalled_source_ids: &[SourceId],
) -> Result<(SourceId, MemoryEnqueueState), rusqlite::Error> {
    let revision_sha256 = conversation_hash(user_text, assistant_text);
    let source_key = format!("turn:{turn_id}");
    let extraction_state = match eligibility {
        MemoryEligibility::Eligible => "pending",
        MemoryEligibility::ArchiveOnly => "not_applicable",
    };
    transaction.execute(
        "INSERT INTO memory_sources (
            kind, source_key, revision_sha256, turn_id, created_at_unix_ms,
            state, extraction_state
         ) VALUES ('conversation', ?1, ?2, ?3, ?4, 'active', ?5)",
        params![
            source_key,
            revision_sha256,
            turn_id,
            created_at_unix_ms,
            extraction_state
        ],
    )?;
    let source_id = SourceId(transaction.last_insert_rowid());
    transaction.execute(
        "INSERT INTO source_parts (source_id, part_index, role, content)
         VALUES (?1, 0, 'user', NULL), (?1, 1, 'assistant', NULL)",
        [source_id.0],
    )?;

    let memory = if eligibility == MemoryEligibility::Eligible {
        let mut dependencies_valid = recalled_source_ids.len() <= MAX_RECALLED_SOURCES;
        let mut seen = HashSet::new();
        for dependency_id in recalled_source_ids.iter().take(MAX_RECALLED_SOURCES) {
            if !seen.insert(*dependency_id) {
                continue;
            }
            if dependency_id.0 <= 0 || dependency_id.0 >= source_id.0 {
                dependencies_valid = false;
                continue;
            }
            let dependency: Option<String> = transaction
                .query_row(
                    "SELECT state FROM memory_sources WHERE id = ?1",
                    [dependency_id.0],
                    |row| row.get(0),
                )
                .optional()?;
            if let Some(state) = dependency {
                transaction.execute(
                    "INSERT OR IGNORE INTO source_dependencies(source_id, depends_on_source_id)
                     VALUES (?1, ?2)",
                    params![source_id.0, dependency_id.0],
                )?;
                dependencies_valid &=
                    state == "active" && !source_is_suppressed(transaction, *dependency_id)?;
            } else {
                dependencies_valid = false;
            }
        }
        if dependencies_valid {
            enqueue_source(transaction, source_id, &revision_sha256, created_at_unix_ms)?
        } else {
            transaction.execute(
                "UPDATE memory_sources SET state = 'suppressed', extraction_state = 'suppressed'
                 WHERE id = ?1",
                [source_id.0],
            )?;
            transaction.execute(
                "INSERT INTO source_suppressions(source_id, suppressed_at_unix_ms, reason)
                 VALUES (?1, ?2, 'forgotten_source')",
                params![source_id.0, created_at_unix_ms],
            )?;
            transaction.execute(
                "UPDATE source_parts SET projection_state = 'suppressed' WHERE source_id = ?1",
                [source_id.0],
            )?;
            MemoryEnqueueState::Suppressed
        }
    } else {
        MemoryEnqueueState::NotEligible
    };
    Ok((source_id, memory))
}

pub(super) fn enqueue_source(
    transaction: &rusqlite::Transaction<'_>,
    source_id: SourceId,
    revision_sha256: &str,
    now: i64,
) -> Result<MemoryEnqueueState, rusqlite::Error> {
    let outstanding: i64 = transaction.query_row(
        "SELECT COUNT(*) FROM memory_jobs WHERE status IN ('queued', 'running')",
        [],
        |row| row.get(0),
    )?;
    if outstanding >= MAX_SOURCE_JOBS {
        transaction.execute(
            "UPDATE memory_sources SET extraction_state = 'pending' WHERE id = ?1",
            [source_id.0],
        )?;
        return Ok(MemoryEnqueueState::Pending);
    }
    let revision = revision_sha256.to_owned();
    let task_key = format!("extract:{}:{revision}", source_id.0);
    let existing: Option<(i64, String)> = transaction
        .query_row(
            "SELECT id, status FROM memory_jobs WHERE task_key = ?1",
            [&task_key],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    if let Some((job_id, status)) = existing {
        if status == "failed" {
            let attempts: i64 = transaction.query_row(
                "SELECT attempt_count FROM memory_jobs WHERE id = ?1",
                [job_id],
                |row| row.get(0),
            )?;
            if attempts >= 3 {
                transaction.execute(
                    "UPDATE memory_sources SET extraction_state = 'failed' WHERE id = ?1",
                    [source_id.0],
                )?;
                return Ok(MemoryEnqueueState::Pending);
            }
            transaction.execute(
                "UPDATE memory_jobs SET status = 'queued', failure_category = NULL,
                     updated_at_unix_ms = ?2 WHERE id = ?1",
                params![job_id, now],
            )?;
        }
    } else {
        transaction.execute(
            "INSERT INTO memory_jobs (
                task_key, source_id, revision_sha256, status, created_at_unix_ms, updated_at_unix_ms
             ) VALUES (?1, ?2, ?3, 'queued', ?4, ?4)",
            params![task_key, source_id.0, revision, now],
        )?;
    }
    transaction.execute(
        "UPDATE memory_sources SET extraction_state = 'queued' WHERE id = ?1",
        [source_id.0],
    )?;
    Ok(MemoryEnqueueState::Queued)
}

pub(super) fn load_source(
    connection: &rusqlite::Connection,
    id: SourceId,
) -> Result<Option<SourceMaterial>, rusqlite::Error> {
    let header = connection
        .query_row(
            "SELECT kind, source_key, revision_sha256, created_at_unix_ms,
                    display_source, turn_id, state
             FROM memory_sources WHERE id = ?1",
            [id.0],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<i64>>(5)?,
                    row.get::<_, String>(6)?,
                ))
            },
        )
        .optional()?;
    let Some((
        kind,
        source_key,
        revision_sha256,
        created_at_unix_ms,
        display_source,
        turn_id,
        state,
    )) = header
    else {
        return Ok(None);
    };
    if state != "active" || source_is_suppressed(connection, id)? {
        return Ok(None);
    }
    let kind = parse_source_kind_sql(kind.as_str())?;
    let mut statement = connection.prepare(
        "SELECT part_index, role, content FROM source_parts
         WHERE source_id = ?1 ORDER BY part_index",
    )?;
    let part_rows = statement
        .query_map([id.0], |row| {
            Ok((
                row.get::<_, u32>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut parts = Vec::with_capacity(part_rows.len());
    for (index, role, content) in part_rows {
        let role = parse_source_role_sql(role.as_str())?;
        let text = match content {
            Some(content) => content,
            None => {
                let Some(turn_id) = turn_id else {
                    continue;
                };
                let column = match role {
                    SourcePartRole::User => "user_text",
                    SourcePartRole::Assistant => "assistant_text",
                    _ => continue,
                };
                connection.query_row(
                    &format!("SELECT {column} FROM turns WHERE id = ?1"),
                    [turn_id],
                    |row| row.get(0),
                )?
            }
        };
        parts.push(SourcePart { index, role, text });
    }
    Ok(Some(SourceMaterial {
        id,
        kind,
        source_key,
        revision_sha256,
        created_at_unix_ms,
        display_source,
        parts,
    }))
}

pub(super) fn load_source_part(
    connection: &rusqlite::Connection,
    source_id: SourceId,
    part_index: u32,
) -> Result<Option<(SourcePartRole, String)>, rusqlite::Error> {
    let source = load_source(connection, source_id)?;
    Ok(source.and_then(|source| {
        source
            .parts
            .into_iter()
            .find(|part| part.index == part_index)
            .map(|part| (part.role, part.text))
    }))
}

pub(super) fn source_is_suppressed(
    connection: &rusqlite::Connection,
    id: SourceId,
) -> Result<bool, rusqlite::Error> {
    connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM source_suppressions WHERE source_id = ?1)",
        [id.0],
        |row| row.get(0),
    )
}

pub(super) fn source_revision_is_active(
    connection: &rusqlite::Connection,
    id: SourceId,
    revision: &str,
) -> Result<bool, rusqlite::Error> {
    connection.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM memory_sources s
            WHERE s.id = ?1 AND s.revision_sha256 = ?2 AND s.state = 'active'
              AND NOT EXISTS (
                  SELECT 1 FROM source_suppressions ss WHERE ss.source_id = s.id
              )
        )",
        params![id.0, revision],
        |row| row.get(0),
    )
}

fn insert_source_chunk(
    transaction: &rusqlite::Transaction<'_>,
    source_id: SourceId,
    revision: &str,
    part_index: u32,
    role: SourcePartRole,
    chunk: SourceChunk,
) -> Result<(), rusqlite::Error> {
    let kind = if role == SourcePartRole::Imported {
        MemoryRecordKind::ImportedChunk
    } else {
        MemoryRecordKind::SourceExcerpt
    };
    let now = unix_time_ms().map_err(|_| rusqlite::Error::InvalidQuery)?;
    let record_key = format!(
        "chunk:{}:{}:{}:{part_index}:{}:{}",
        source_id.0,
        revision,
        role.as_str(),
        chunk.start_byte,
        chunk.end_byte
    );
    // LEARNING: A correction suppresses only the overlapping projection, not
    // the immutable source text. This check also protects chunks inserted by
    // a later bounded backfill page after the correction was committed.
    let overlaps_superseded_fact = role == SourcePartRole::User
        && transaction.query_row(
            "SELECT EXISTS(
                SELECT 1
                FROM memory_evidence fact
                JOIN memory_records old_fact ON old_fact.id = fact.record_id
                JOIN memory_records correction
                  ON correction.supersedes_id = old_fact.id
                 AND correction.correction_state = 'applied'
                WHERE old_fact.status = 'superseded'
                  AND old_fact.attribution = 'user_statement'
                  AND fact.source_id = ?1 AND fact.part_index = ?2
                  AND fact.start_byte < ?4 AND fact.end_byte > ?3
            )",
            params![source_id.0, part_index, chunk.start_byte, chunk.end_byte,],
            |row| row.get::<_, bool>(0),
        )?;
    let (record_status, projection_op) = if overlaps_superseded_fact {
        ("superseded", "delete")
    } else {
        ("active", "upsert")
    };
    transaction.execute(
        "INSERT OR IGNORE INTO memory_records (
            source_id, record_key, text, record_kind, attribution, status,
            created_at_unix_ms, projection_generation, projected_generation,
            projection_op
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1, 0, ?8)",
        params![
            source_id.0,
            record_key,
            chunk.text,
            kind.as_str(),
            role.attribution().as_str(),
            record_status,
            now,
            projection_op,
        ],
    )?;
    let record_id: i64 = transaction.query_row(
        "SELECT id FROM memory_records WHERE record_key = ?1",
        [record_key],
        |row| row.get(0),
    )?;
    transaction.execute(
        "INSERT OR IGNORE INTO memory_evidence (
            record_id, source_id, part_index, start_byte, end_byte, quote
         ) VALUES (?1, ?2, ?3, ?4, ?5, NULL)",
        params![
            record_id,
            source_id.0,
            part_index,
            chunk.start_byte,
            chunk.end_byte,
        ],
    )?;
    Ok(())
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub(super) fn conversation_hash(user: &str, assistant: &str) -> String {
    let mut digest = Sha256::new();
    for (role, text) in [("user", user), ("assistant", assistant)] {
        digest.update((role.len() as u64).to_be_bytes());
        digest.update(role.as_bytes());
        digest.update((text.len() as u64).to_be_bytes());
        digest.update(text.as_bytes());
    }
    format!("{:x}", digest.finalize())
}

pub(super) fn parse_source_kind(value: &str) -> Result<SourceKind, rusqlite::Error> {
    match value {
        "conversation" => Ok(SourceKind::Conversation),
        "import" => Ok(SourceKind::Import),
        "action" => Ok(SourceKind::Action),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}

pub(super) fn parse_source_role(value: &str) -> Result<SourcePartRole, rusqlite::Error> {
    SourcePartRole::from_str(value).ok_or(rusqlite::Error::InvalidQuery)
}

fn parse_source_kind_sql(value: &str) -> Result<SourceKind, rusqlite::Error> {
    parse_source_kind(value)
}

fn parse_source_role_sql(value: &str) -> Result<SourcePartRole, rusqlite::Error> {
    parse_source_role(value)
}
