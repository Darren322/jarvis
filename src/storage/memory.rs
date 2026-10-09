use rusqlite::{OptionalExtension, TransactionBehavior, params};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use tokio_rusqlite::Connection;

use super::{
    CommitDisposition, CorrectionEvidence, CorrectionMarker, CorrectionState, ForgetReceipt,
    HydratedMemory, MemoryAttribution, MemoryEligibility, MemoryEvidence, MemoryId,
    MemoryListFilter, MemoryRecordKind, MemoryRecordStatus, MemoryStats, MemorySummary,
    PendingSourcePart, SourceChunk, SourceId, SourceKind, SourceMaterial, SourcePart,
    SourcePartRole, SourceReceipt, SourceReceiptState, StorageError,
};

pub(super) const MAX_SOURCE_JOBS: i64 = 128;
const MAX_MEMORY_BYTES: usize = 512;
const MAX_EXTRACTION_RECORDS: usize = 32;
const MAX_EVIDENCE_PER_RECORD: usize = 8;
const MAX_EVIDENCE_QUOTE_BYTES: usize = 512;
pub(super) const MAX_RECALLED_SOURCES: usize = 32;
const MAX_CORRECTION_CANDIDATES: usize = 33;
const MAX_FORGET_RECEIPT_ITEMS: usize = 128;
const MAX_IMPORT_BYTES: usize = 1024 * 1024;
pub(super) const MAX_RECORD_BYTES: usize = 4096;
const MAX_LIST_ITEMS: usize = 50;
const MAX_SOURCE_PAGE: usize = 32;

#[derive(Clone)]
pub(crate) struct MemoryRepository {
    pub(super) connection: Connection,
}

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
        let created_at_unix_ms = super::unix_time_ms()?;
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

    pub(crate) async fn list_memories(
        &self,
        filter: MemoryListFilter,
    ) -> Result<Vec<MemorySummary>, StorageError> {
        let limit = filter.limit.clamp(1, MAX_LIST_ITEMS) as i64;
        let query = filter.query;
        let status = filter.status.map(|status| status.as_str().to_owned());
        let result = self
            .connection
            .call(move |connection| {
                let mut statement = connection.prepare(
                    "SELECT m.id, m.source_id, m.text, m.record_kind, m.attribution, m.status,
                            m.correction_state, m.supersedes_id,
                            s.kind, s.display_source, m.created_at_unix_ms,
                            s.created_at_unix_ms
                     FROM memory_records m
                     JOIN memory_sources s ON s.id = m.source_id
                     WHERE (?1 IS NULL OR instr(lower(m.text), lower(?1)) > 0)
                       AND (?2 IS NULL OR m.status = ?2)
                     ORDER BY m.id DESC LIMIT ?3",
                )?;
                let mut rows = statement.query(params![query, status, limit])?;
                let mut memories = Vec::new();
                while let Some(row) = rows.next()? {
                    memories.push(MemorySummary {
                        id: MemoryId(row.get(0)?),
                        source_id: SourceId(row.get(1)?),
                        text: row.get(2)?,
                        kind: parse_record_kind(row.get::<_, String>(3)?.as_str())?,
                        attribution: parse_attribution(row.get::<_, String>(4)?.as_str())?,
                        status: parse_record_status(row.get::<_, String>(5)?.as_str())?,
                        correction_state: parse_correction_state(
                            row.get::<_, String>(6)?.as_str(),
                        )?,
                        supersedes_id: row.get::<_, Option<i64>>(7)?.map(MemoryId),
                        source_kind: parse_source_kind(row.get::<_, String>(8)?.as_str())?,
                        source_name: row.get(9)?,
                        created_at_unix_ms: row.get(10)?,
                        source_created_at_unix_ms: row.get(11)?,
                    });
                }
                Ok(memories)
            })
            .await?;
        Ok(result)
    }

    pub(crate) async fn stats(&self) -> Result<MemoryStats, StorageError> {
        let result = self
            .connection
            .call(|connection| {
                let cursor: i64 = connection.query_row(
                    "SELECT integer_value FROM memory_state WHERE key = 'backfill_cursor_turn_id'",
                    [],
                    |row| row.get(0),
                )?;
                let backfill_complete = !connection.query_row(
                    "SELECT EXISTS(SELECT 1 FROM turns WHERE id > ?1)",
                    [cursor],
                    |row| row.get::<_, bool>(0),
                )?;
                Ok(MemoryStats {
                    queued_extractions: scalar_count(
                        connection,
                        "SELECT COUNT(*) FROM memory_jobs WHERE status = 'queued'",
                    )?,
                    running_extractions: scalar_count(
                        connection,
                        "SELECT COUNT(*) FROM memory_jobs WHERE status = 'running'",
                    )?,
                    failed_extractions: scalar_count(
                        connection,
                        "SELECT COUNT(*) FROM memory_jobs WHERE status = 'failed'",
                    )?,
                    pending_sources: scalar_count(
                        connection,
                        "SELECT COUNT(*) FROM memory_sources WHERE extraction_state = 'pending'",
                    )?,
                    pending_source_parts: scalar_count(
                        connection,
                        "SELECT COUNT(*) FROM source_parts WHERE projection_state = 'pending'",
                    )?,
                    pending_projections: scalar_count(
                        connection,
                        "SELECT COUNT(*) FROM memory_records WHERE projected_generation < projection_generation",
                    )?,
                    backfill_complete,
                })
            })
            .await?;
        Ok(result)
    }

    /// Marks all durable records for a new search index generation without
    /// loading the corpus into process memory; the worker consumes bounded pages.
    pub(crate) async fn schedule_projection_rebuild(&self) -> Result<usize, StorageError> {
        let now = super::unix_time_ms()?;
        let changed = self
            .connection
            .call(move |connection| {
                connection.execute(
                    "UPDATE memory_records
                     SET projection_generation = projection_generation + 1,
                         projection_op = CASE
                             WHEN status = 'active'
                               AND EXISTS (
                                   SELECT 1 FROM memory_sources s
                                   WHERE s.id = memory_records.source_id AND s.state = 'active'
                               )
                               AND NOT EXISTS (
                                   SELECT 1 FROM source_suppressions ss
                                   WHERE ss.source_id = memory_records.source_id
                               ) THEN 'upsert'
                             ELSE 'delete'
                         END,
                         projection_lease_token = projection_lease_token + 1,
                         projection_lease_until_unix_ms = NULL,
                         projection_retry_after_unix_ms = ?1",
                    [now],
                )
            })
            .await?;
        Ok(changed)
    }

    pub(crate) async fn hydrate_active(
        &self,
        ids: &[MemoryId],
    ) -> Result<Vec<HydratedMemory>, StorageError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let ids = ids.iter().take(100).map(|id| id.0).collect::<Vec<_>>();
        let result = self
            .connection
            .call(move |connection| {
                let placeholders = std::iter::repeat_n("?", ids.len())
                    .collect::<Vec<_>>()
                    .join(",");
                let sql = format!(
                    "SELECT m.id, m.text, m.record_kind, m.attribution, m.status,
                            m.correction_state, m.supersedes_id, s.created_at_unix_ms
                     FROM memory_records m
                     JOIN memory_sources s ON s.id = m.source_id
                     WHERE m.id IN ({placeholders}) AND m.status = 'active'
                       AND m.projection_op = 'upsert' AND s.state = 'active'
                       AND NOT EXISTS (
                           SELECT 1 FROM source_suppressions ss WHERE ss.source_id = s.id
                       )
                     ORDER BY m.id"
                );
                let mut statement = connection.prepare(&sql)?;
                let base = statement
                    .query_map(rusqlite::params_from_iter(ids.iter()), |row| {
                        Ok((
                            MemoryId(row.get(0)?),
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, String>(4)?,
                            row.get::<_, String>(5)?,
                            row.get::<_, Option<i64>>(6)?,
                            row.get::<_, i64>(7)?,
                        ))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                let mut hydrated = Vec::new();
                for (
                    id,
                    text,
                    kind,
                    attribution,
                    status,
                    correction_state,
                    supersedes_id,
                    source_created_at_unix_ms,
                ) in base
                {
                    let mut evidence_statement = connection.prepare(
                        "SELECT e.source_id, e.part_index, s.kind, s.display_source, sp.role,
                                e.start_byte, e.end_byte, e.quote, s.created_at_unix_ms
                         FROM memory_evidence e
                         JOIN memory_sources s ON s.id = e.source_id
                         JOIN source_parts sp
                           ON sp.source_id = e.source_id AND sp.part_index = e.part_index
                         WHERE e.record_id = ?1 AND s.state = 'active'
                           AND NOT EXISTS (
                               SELECT 1 FROM source_suppressions ss WHERE ss.source_id = s.id
                           )
                         ORDER BY e.source_id, e.part_index, e.start_byte",
                    )?;
                    let evidence = evidence_statement
                        .query_map([id.0], |row| {
                            Ok(MemoryEvidence {
                                source_id: SourceId(row.get(0)?),
                                part_index: row.get(1)?,
                                source_kind: parse_source_kind(row.get::<_, String>(2)?.as_str())?,
                                source_name: row.get(3)?,
                                role: parse_source_role(row.get::<_, String>(4)?.as_str())?,
                                start_byte: row.get(5)?,
                                end_byte: row.get(6)?,
                                quote: row.get(7)?,
                                source_created_at_unix_ms: row.get(8)?,
                            })
                        })?
                        .collect::<rusqlite::Result<Vec<_>>>()?;
                    hydrated.push(HydratedMemory {
                        id,
                        text,
                        kind: parse_record_kind(&kind)?,
                        attribution: parse_attribution(&attribution)?,
                        status: parse_record_status(&status)?,
                        correction_state: parse_correction_state(&correction_state)?,
                        supersedes_id: supersedes_id.map(MemoryId),
                        source_created_at_unix_ms,
                        evidence,
                    });
                }
                Ok(hydrated)
            })
            .await?;
        Ok(result)
    }

    pub(crate) async fn forget(&self, id: MemoryId) -> Result<ForgetReceipt, StorageError> {
        let at = super::unix_time_ms()?;
        let receipt = self
            .connection
            .call(move |connection| {
                let transaction =
                    connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let source_ids = {
                    let mut statement = transaction.prepare(
                        "SELECT DISTINCT source_id FROM memory_evidence WHERE record_id = ?1",
                    )?;
                    statement
                        .query_map([id.0], |row| Ok(SourceId(row.get(0)?)))?
                        .collect::<rusqlite::Result<Vec<_>>>()?
                };
                if source_ids.is_empty() {
                    transaction.commit()?;
                    return Ok(ForgetReceipt::default());
                }
                let mut receipt = ForgetReceipt::default();
                for source_id in source_ids {
                    suppress_source(
                        &transaction,
                        source_id,
                        at,
                        "forgotten_memory",
                        &mut receipt,
                    )?;
                }
                transaction.commit()?;
                Ok(receipt)
            })
            .await?;
        Ok(receipt)
    }

    pub(crate) async fn forget_source(
        &self,
        source_id: SourceId,
    ) -> Result<ForgetReceipt, StorageError> {
        let at = super::unix_time_ms()?;
        let receipt = self
            .connection
            .call(move |connection| {
                let transaction =
                    connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let mut receipt = ForgetReceipt::default();
                suppress_source(
                    &transaction,
                    source_id,
                    at,
                    "forgotten_source",
                    &mut receipt,
                )?;
                transaction.commit()?;
                Ok(receipt)
            })
            .await?;
        Ok(receipt)
    }
}

pub(super) fn insert_conversation_source(
    transaction: &rusqlite::Transaction<'_>,
    turn_id: i64,
    user_text: &str,
    assistant_text: &str,
    created_at_unix_ms: i64,
    eligibility: MemoryEligibility,
    recalled_source_ids: &[SourceId],
) -> Result<(SourceId, super::MemoryEnqueueState), rusqlite::Error> {
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
            super::MemoryEnqueueState::Suppressed
        }
    } else {
        super::MemoryEnqueueState::NotEligible
    };
    Ok((source_id, memory))
}

pub(super) fn enqueue_source(
    transaction: &rusqlite::Transaction<'_>,
    source_id: SourceId,
    revision_sha256: &str,
    now: i64,
) -> Result<super::MemoryEnqueueState, rusqlite::Error> {
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
        return Ok(super::MemoryEnqueueState::Pending);
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
                return Ok(super::MemoryEnqueueState::Pending);
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
    Ok(super::MemoryEnqueueState::Queued)
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

pub(super) fn insert_source_chunk(
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
    let now = super::unix_time_ms().map_err(|_| rusqlite::Error::InvalidQuery)?;
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

pub(super) fn commit_new_memories(
    transaction: &rusqlite::Transaction<'_>,
    lease_id: i64,
    source_id: SourceId,
    revision: &str,
    records: Vec<super::NewMemory>,
) -> Result<CommitDisposition, rusqlite::Error> {
    if !source_revision_is_active(transaction, source_id, revision)? {
        return Ok(CommitDisposition::Suppressed);
    }
    let source = load_source(transaction, source_id)?;
    let Some(source) = source else {
        return Ok(CommitDisposition::Suppressed);
    };
    if source.kind != SourceKind::Conversation {
        return Ok(CommitDisposition::InvalidEvidence);
    }
    if records.is_empty() {
        return Ok(CommitDisposition::Empty);
    }

    let source_parts = source
        .parts
        .iter()
        .map(|part| (part.index, (part.role, part.text.as_str())))
        .collect::<std::collections::HashMap<_, _>>();
    if records.len() > MAX_EXTRACTION_RECORDS {
        return Ok(CommitDisposition::InvalidEvidence);
    }
    for record in &records {
        if record.text.trim().is_empty()
            || record.text.len() > MAX_MEMORY_BYTES
            || record.evidence.is_empty()
            || record.evidence.len() > MAX_EVIDENCE_PER_RECORD
        {
            return Ok(CommitDisposition::InvalidEvidence);
        }
        let mut roles = Vec::with_capacity(record.evidence.len());
        for evidence in &record.evidence {
            if evidence.quote.len() > MAX_EVIDENCE_QUOTE_BYTES {
                return Ok(CommitDisposition::InvalidEvidence);
            }
            let Some((_, text)) = source_parts.get(&evidence.part_index) else {
                return Ok(CommitDisposition::InvalidEvidence);
            };
            let Some(span) = text.get(evidence.start_byte as usize..evidence.end_byte as usize)
            else {
                return Ok(CommitDisposition::InvalidEvidence);
            };
            if span.is_empty() || span != evidence.quote {
                return Ok(CommitDisposition::InvalidEvidence);
            }
            roles.push(source_parts[&evidence.part_index].0);
        }
        let attribution_matches = match record.attribution {
            MemoryAttribution::UserStatement => {
                roles.iter().all(|role| *role == SourcePartRole::User)
            }
            MemoryAttribution::AssistantAnswer => {
                roles.iter().all(|role| *role == SourcePartRole::Assistant)
            }
            MemoryAttribution::ImportedClaim => {
                roles.iter().all(|role| *role == SourcePartRole::Imported)
            }
            MemoryAttribution::ActionResult => roles
                .iter()
                .all(|role| *role == SourcePartRole::ActionResult),
            MemoryAttribution::Inference => true,
        };
        if !attribution_matches {
            return Ok(CommitDisposition::InvalidEvidence);
        }
    }

    // Resolve corrections while holding the transaction. Only a uniquely
    // grounded prior user statement can supersede; ambiguous proposals remain
    // visible as NeedsReview and keep their conflicting provenance active.
    let mut reserved_targets = HashSet::new();
    let mut correction_decisions = Vec::with_capacity(records.len());
    for record in &records {
        let decision = if record.correction_needs_review {
            CorrectionDecision::needs_review()
        } else if let Some(correction) = &record.correction {
            if record.attribution != MemoryAttribution::UserStatement
                || !current_correction_is_grounded(&source_parts, record, correction)
            {
                CorrectionDecision::needs_review()
            } else {
                let candidates =
                    find_correction_candidates(transaction, source_id, record.kind, correction)?;
                match candidates.as_slice() {
                    [candidate] if !reserved_targets.contains(candidate) => {
                        reserved_targets.insert(*candidate);
                        CorrectionDecision {
                            state: CorrectionState::Applied,
                            supersedes_id: Some(MemoryId(*candidate)),
                        }
                    }
                    _ => CorrectionDecision::needs_review(),
                }
            }
        } else {
            CorrectionDecision {
                state: CorrectionState::None,
                supersedes_id: None,
            }
        };
        correction_decisions.push(decision);
    }

    let now = super::unix_time_ms().map_err(|_| rusqlite::Error::InvalidQuery)?;
    for (index, record) in records.into_iter().enumerate() {
        let record_key = format!("extract:{lease_id}:{index}");
        let correction = correction_decisions[index];
        transaction.execute(
            "INSERT INTO memory_records (
                source_id, record_key, text, record_kind, attribution, status,
                correction_state, supersedes_id, created_at_unix_ms,
                projection_generation, projected_generation, projection_op
             ) VALUES (?1, ?2, ?3, ?4, ?5, 'active', ?6, ?7, ?8, 1, 0, 'upsert')",
            params![
                source_id.0,
                record_key,
                record.text,
                record.kind.as_str(),
                record.attribution.as_str(),
                correction.state.as_str(),
                correction.supersedes_id.map(|id| id.0),
                now,
            ],
        )?;
        let memory_id = transaction.last_insert_rowid();
        for evidence in record.evidence {
            transaction.execute(
                "INSERT INTO memory_evidence (
                    record_id, source_id, part_index, start_byte, end_byte, quote
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    memory_id,
                    source_id.0,
                    evidence.part_index,
                    evidence.start_byte,
                    evidence.end_byte,
                    evidence.quote,
                ],
            )?;
        }
        if let Some(superseded_id) = correction.supersedes_id {
            supersede_corrected_memory(transaction, superseded_id, now)?;
        }
    }
    Ok(CommitDisposition::Committed)
}

#[derive(Clone, Copy)]
struct CorrectionDecision {
    state: CorrectionState,
    supersedes_id: Option<MemoryId>,
}

impl CorrectionDecision {
    fn needs_review() -> Self {
        Self {
            state: CorrectionState::NeedsReview,
            supersedes_id: None,
        }
    }
}

fn current_correction_is_grounded(
    source_parts: &std::collections::HashMap<u32, (SourcePartRole, &str)>,
    record: &super::NewMemory,
    correction: &CorrectionEvidence,
) -> bool {
    let spans = [
        &correction.subject,
        &correction.property,
        &correction.old_value,
        &correction.new_value,
    ];
    let Some((role, source_text)) = source_parts.get(&correction.subject.part_index) else {
        return false;
    };
    if *role != SourcePartRole::User
        || spans
            .iter()
            .any(|span| span.part_index != correction.subject.part_index)
    {
        return false;
    }
    for span in spans {
        let Some(literal) = source_text.get(span.start_byte as usize..span.end_byte as usize)
        else {
            return false;
        };
        if span.start_byte >= span.end_byte
            || literal != span.quote
            || !record.evidence.iter().any(|evidence| {
                evidence.part_index == span.part_index
                    && evidence.start_byte <= span.start_byte
                    && evidence.end_byte >= span.end_byte
            })
        {
            return false;
        }
    }
    if !matches!(
        correction.subject.quote.to_ascii_lowercase().as_str(),
        "i" | "my"
    ) || correction.property.quote.trim().is_empty()
        || correction.old_value.quote.trim().is_empty()
        || correction.new_value.quote.trim().is_empty()
        || correction
            .old_value
            .quote
            .trim()
            .eq_ignore_ascii_case(correction.new_value.quote.trim())
        || correction.old_value.start_byte < correction.property.end_byte
        || correction.new_value.start_byte < correction.property.end_byte
        || !adjacent_by_whitespace(
            source_text,
            correction.subject.end_byte,
            correction.property.start_byte,
        )
        || is_inside_quotes(source_text, correction.subject.start_byte as usize)
        || !has_first_person_assertion_prefix(
            source_text,
            correction.subject.start_byte as usize,
            Some(correction.marker),
        )
    {
        return false;
    }

    let span_start = correction.subject.start_byte;
    let span_end = correction
        .old_value
        .end_byte
        .max(correction.new_value.end_byte);
    let Some(evidence_quote) = record.evidence.iter().find(|evidence| {
        evidence.part_index == correction.subject.part_index
            && evidence.start_byte <= span_start
            && evidence.end_byte >= span_end
    }) else {
        return false;
    };
    if !contains_phrase(&evidence_quote.quote, correction.marker.phrase()) {
        return false;
    }
    true
}

fn find_correction_candidates(
    transaction: &rusqlite::Transaction<'_>,
    source_id: SourceId,
    kind: MemoryRecordKind,
    correction: &CorrectionEvidence,
) -> Result<Vec<i64>, rusqlite::Error> {
    let ids = {
        let mut statement = transaction.prepare(
            "SELECT DISTINCT m.id
             FROM memory_records m
             JOIN memory_sources s ON s.id = m.source_id
             WHERE m.source_id < ?1 AND m.record_kind = ?2
               AND m.attribution = 'user_statement' AND m.status = 'active'
               AND s.kind = 'conversation' AND s.state = 'active'
               AND NOT EXISTS (
                   SELECT 1 FROM source_suppressions ss WHERE ss.source_id = s.id
               )
               AND EXISTS (
                   SELECT 1 FROM memory_evidence e
                   JOIN source_parts sp
                     ON sp.source_id = e.source_id AND sp.part_index = e.part_index
                   WHERE e.record_id = m.id AND sp.role = 'user'
                     AND e.quote IS NOT NULL
                     AND instr(e.quote, ?3) > 0 AND instr(e.quote, ?4) > 0
               )
             ORDER BY m.id DESC LIMIT ?5",
        )?;
        statement
            .query_map(
                params![
                    source_id.0,
                    kind.as_str(),
                    correction.property.quote,
                    correction.old_value.quote,
                    MAX_CORRECTION_CANDIDATES as i64,
                ],
                |row| row.get::<_, i64>(0),
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    if ids.len() >= MAX_CORRECTION_CANDIDATES {
        return Ok(ids);
    }

    let mut candidates = Vec::new();
    for id in ids {
        if correction_matches_prior_memory(transaction, MemoryId(id), correction)? {
            candidates.push(id);
            if candidates.len() > 1 {
                break;
            }
        }
    }
    Ok(candidates)
}

fn correction_matches_prior_memory(
    transaction: &rusqlite::Transaction<'_>,
    memory_id: MemoryId,
    correction: &CorrectionEvidence,
) -> Result<bool, rusqlite::Error> {
    let evidence = {
        let mut statement = transaction.prepare(
            "SELECT e.source_id, e.part_index, e.start_byte, e.end_byte, e.quote
             FROM memory_evidence e
             JOIN source_parts sp
               ON sp.source_id = e.source_id AND sp.part_index = e.part_index
             WHERE e.record_id = ?1 AND sp.role = 'user' AND e.quote IS NOT NULL
             LIMIT 8",
        )?;
        statement
            .query_map([memory_id.0], |row| {
                Ok((
                    SourceId(row.get(0)?),
                    row.get::<_, u32>(1)?,
                    row.get::<_, u32>(2)?,
                    row.get::<_, u32>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    for (source_id, part_index, start_byte, end_byte, quote) in evidence {
        let Some((role, source_text)) = load_source_part(transaction, source_id, part_index)?
        else {
            continue;
        };
        if role != SourcePartRole::User
            || source_text
                .get(start_byte as usize..end_byte as usize)
                .is_none_or(|slice| slice != quote)
        {
            continue;
        }
        if context_has_property_value(
            &quote,
            &source_text,
            start_byte,
            &correction.subject.quote,
            &correction.property.quote,
            &correction.old_value.quote,
        ) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn context_has_property_value(
    quote: &str,
    full_source: &str,
    quote_start: u32,
    subject: &str,
    property: &str,
    value: &str,
) -> bool {
    for subject_start in phrase_positions(quote, subject) {
        let subject_end = subject_start + subject.len();
        let absolute_subject_start = quote_start as usize + subject_start;
        if is_inside_quotes(full_source, absolute_subject_start)
            || !has_first_person_assertion_prefix(full_source, absolute_subject_start, None)
        {
            continue;
        }
        for property_start in phrase_positions(quote, property) {
            if property_start < subject_end
                || !adjacent_by_whitespace(quote, subject_end as u32, property_start as u32)
            {
                continue;
            }
            let property_end = property_start + property.len();
            if phrase_positions(quote, value)
                .into_iter()
                .any(|value_start| {
                    value_start >= property_end
                        && value_gap_is_plain(quote, property_end, value_start)
                })
            {
                return true;
            }
        }
    }
    false
}

fn value_gap_is_plain(text: &str, start: usize, end: usize) -> bool {
    let Some(gap) = text.get(start..end) else {
        return false;
    };
    let gap = gap.trim().to_ascii_lowercase();
    gap.is_empty()
        || matches!(
            gap.as_str(),
            "is" | "are" | "was" | "were" | "am" | ":" | "="
        )
}

fn adjacent_by_whitespace(text: &str, left_end: u32, right_start: u32) -> bool {
    if right_start < left_end {
        return false;
    }
    text.get(left_end as usize..right_start as usize)
        .is_some_and(|between| between.chars().all(char::is_whitespace))
}

fn contains_phrase(text: &str, phrase: &str) -> bool {
    phrase_positions(text, phrase).into_iter().any(|start| {
        let end = start + phrase.len();
        let before_is_word = text[..start]
            .chars()
            .next_back()
            .is_some_and(char::is_alphanumeric);
        let after_is_word = text[end..]
            .chars()
            .next()
            .is_some_and(char::is_alphanumeric);
        !before_is_word && !after_is_word
    })
}

fn phrase_positions(text: &str, phrase: &str) -> Vec<usize> {
    if phrase.is_empty() || phrase.len() > text.len() {
        return Vec::new();
    }
    text.char_indices()
        .filter_map(|(start, _)| {
            text.get(start..start + phrase.len())
                .filter(|candidate| candidate.eq_ignore_ascii_case(phrase))
                .map(|_| start)
        })
        .collect()
}

fn has_first_person_assertion_prefix(
    text: &str,
    byte_offset: usize,
    required_marker: Option<CorrectionMarker>,
) -> bool {
    let Some(prefix) = text.get(..byte_offset) else {
        return false;
    };
    let line_start = prefix.rfind(['\n', '\r']).map_or(0, |index| index + 1);
    let preceding_line_text = &prefix[line_start..];
    let preceding = preceding_line_text.trim_start();
    if preceding.trim().is_empty() {
        return true;
    }

    const MARKERS: [CorrectionMarker; 7] = [
        CorrectionMarker::InsteadOf,
        CorrectionMarker::RatherThan,
        CorrectionMarker::NoLonger,
        CorrectionMarker::IMeant,
        CorrectionMarker::Actually,
        CorrectionMarker::Correction,
        CorrectionMarker::Instead,
    ];
    MARKERS.into_iter().any(|marker| {
        if required_marker.is_some_and(|required| required != marker) {
            return false;
        }
        let phrase = marker.phrase();
        let Some(found) = preceding.get(..phrase.len()) else {
            return false;
        };
        if !found.eq_ignore_ascii_case(phrase) {
            return false;
        }
        let suffix = &preceding[phrase.len()..];
        if suffix.chars().next().is_some_and(char::is_alphanumeric) {
            return false;
        }
        suffix.chars().all(|character| {
            character.is_whitespace()
                || character.is_ascii_punctuation()
                || matches!(character, '—' | '–')
        })
    })
}

fn is_inside_quotes(text: &str, byte_offset: usize) -> bool {
    let mut double_quoted = false;
    let mut single_quoted = false;
    for (index, character) in text.char_indices() {
        if index >= byte_offset {
            break;
        }
        match character {
            '"' => double_quoted = !double_quoted,
            '“' => double_quoted = true,
            '”' => double_quoted = false,
            '\'' => {
                let before_is_word = text[..index]
                    .chars()
                    .next_back()
                    .is_some_and(char::is_alphanumeric);
                let after_is_word = text[index + character.len_utf8()..]
                    .chars()
                    .next()
                    .is_some_and(char::is_alphanumeric);
                if !(before_is_word && after_is_word) {
                    single_quoted = !single_quoted;
                }
            }
            '‘' => single_quoted = true,
            '’' => {
                let before_is_word = text[..index]
                    .chars()
                    .next_back()
                    .is_some_and(char::is_alphanumeric);
                let after_is_word = text[index + character.len_utf8()..]
                    .chars()
                    .next()
                    .is_some_and(char::is_alphanumeric);
                if !(before_is_word && after_is_word) {
                    single_quoted = false;
                }
            }
            _ => {}
        }
    }
    double_quoted || single_quoted
}

fn supersede_corrected_memory(
    transaction: &rusqlite::Transaction<'_>,
    memory_id: MemoryId,
    now: i64,
) -> Result<(), rusqlite::Error> {
    transaction.execute(
        "UPDATE memory_records
         SET status = 'superseded', projection_generation = projection_generation + 1,
             projection_op = 'delete', projection_lease_token = projection_lease_token + 1,
             projection_lease_until_unix_ms = NULL, projection_retry_after_unix_ms = ?2
         WHERE id = ?1 AND status = 'active'",
        params![memory_id.0, now],
    )?;
    transaction.execute(
        "UPDATE memory_records
         SET status = 'superseded', projection_generation = projection_generation + 1,
             projection_op = 'delete', projection_lease_token = projection_lease_token + 1,
             projection_lease_until_unix_ms = NULL, projection_retry_after_unix_ms = ?2
         WHERE status = 'active' AND record_kind = 'source_excerpt'
           AND attribution = 'user_statement'
           AND id IN (
               SELECT chunk.record_id FROM memory_evidence chunk
               JOIN source_parts sp ON sp.source_id = chunk.source_id
                    AND sp.part_index = chunk.part_index AND sp.role = 'user'
               WHERE EXISTS (
                   SELECT 1 FROM memory_evidence fact
                   WHERE fact.record_id = ?1
                     AND fact.source_id = chunk.source_id
                     AND fact.part_index = chunk.part_index
                     AND fact.start_byte < chunk.end_byte
                     AND fact.end_byte > chunk.start_byte
               )
           )",
        params![memory_id.0, now],
    )?;
    Ok(())
}

fn supersede_other_revisions(
    transaction: &rusqlite::Transaction<'_>,
    kind: &str,
    source_key: &str,
    except: SourceId,
    now: i64,
) -> Result<(), rusqlite::Error> {
    let source_ids = {
        let mut statement = transaction.prepare(
            "SELECT id FROM memory_sources
             WHERE kind = ?1 AND source_key = ?2 AND id <> ?3 AND state = 'active'",
        )?;
        statement
            .query_map(params![kind, source_key, except.0], |row| {
                row.get::<_, i64>(0)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    for source_id in source_ids {
        transaction.execute(
            "UPDATE memory_sources SET state = 'superseded'
             WHERE id = ?1 AND state = 'active'",
            [source_id],
        )?;
        transaction.execute(
            "UPDATE memory_records
             SET status = 'superseded', projection_generation = projection_generation + 1,
                 projection_op = 'delete', projection_lease_until_unix_ms = NULL,
                 projection_retry_after_unix_ms = ?2
             WHERE source_id = ?1 AND status = 'active'",
            params![source_id, now],
        )?;
        supersede_derived_sources(transaction, SourceId(source_id), now)?;
    }
    Ok(())
}

fn supersede_derived_sources(
    transaction: &rusqlite::Transaction<'_>,
    source_id: SourceId,
    now: i64,
) -> Result<(), rusqlite::Error> {
    const DESCENDANTS: &str = "WITH RECURSIVE descendants(id) AS (
        SELECT source_id FROM source_dependencies WHERE depends_on_source_id = ?1
        UNION
        SELECT d.source_id FROM source_dependencies d
        JOIN descendants child ON d.depends_on_source_id = child.id
    ) ";
    transaction.execute(
        &format!(
            "{DESCENDANTS}UPDATE memory_sources
             SET state = 'superseded', extraction_state = 'suppressed'
             WHERE id IN (SELECT id FROM descendants) AND state = 'active'"
        ),
        [source_id.0],
    )?;
    transaction.execute(
        &format!(
            "{DESCENDANTS}UPDATE source_parts SET projection_state = 'suppressed'
             WHERE source_id IN (SELECT id FROM descendants)"
        ),
        [source_id.0],
    )?;
    transaction.execute(
        &format!(
            "{DESCENDANTS}UPDATE memory_jobs
             SET status = 'completed', failure_category = 'source_changed',
                 lease_token = lease_token + 1, lease_until_unix_ms = NULL,
                 updated_at_unix_ms = ?2
             WHERE source_id IN (SELECT id FROM descendants)
               AND status IN ('queued', 'running')"
        ),
        params![source_id.0, now],
    )?;
    transaction.execute(
        &format!(
            "{DESCENDANTS}UPDATE memory_records
             SET status = 'superseded', projection_generation = projection_generation + 1,
                 projection_op = 'delete', projection_lease_token = projection_lease_token + 1,
                 projection_lease_until_unix_ms = NULL,
                 projection_retry_after_unix_ms = 0
             WHERE source_id IN (SELECT id FROM descendants) AND status = 'active'"
        ),
        [source_id.0],
    )?;
    Ok(())
}

fn reactivate_source_records(
    transaction: &rusqlite::Transaction<'_>,
    source_id: SourceId,
) -> Result<(), rusqlite::Error> {
    transaction.execute(
        "UPDATE memory_records
         SET status = 'active', projection_generation = projection_generation + 1,
             projection_op = 'upsert', projection_lease_until_unix_ms = NULL,
             projection_retry_after_unix_ms = 0
         WHERE source_id = ?1 AND record_kind IN ('source_excerpt', 'imported_chunk')
           AND status = 'superseded'",
        [source_id.0],
    )?;
    Ok(())
}

fn suppress_source(
    transaction: &rusqlite::Transaction<'_>,
    source_id: SourceId,
    at: i64,
    reason: &str,
    receipt: &mut ForgetReceipt,
) -> Result<(), rusqlite::Error> {
    const AFFECTED: &str = "WITH RECURSIVE affected(id) AS (
        SELECT id FROM memory_sources WHERE id = ?1
        UNION
        SELECT d.source_id FROM source_dependencies d
        JOIN affected a ON d.depends_on_source_id = a.id
    ) ";

    let source_count = affected_count(transaction, AFFECTED, "memory_sources", source_id)?;
    if source_count == 0 {
        return Ok(());
    }
    let turn_count = affected_count(transaction, AFFECTED, "turns", source_id)?;
    let memory_count: i64 = transaction.query_row(
        &format!("{AFFECTED}SELECT COUNT(*) FROM memory_records WHERE source_id IN (SELECT id FROM affected)"),
        [source_id.0],
        |row| row.get(0),
    )?;
    receipt.suppressed_source_count += source_count;
    receipt.affected_turn_count += turn_count;
    receipt.forgotten_memory_count += memory_count.max(0) as usize;
    receipt.ids_truncated |= source_count > MAX_FORGET_RECEIPT_ITEMS
        || turn_count > MAX_FORGET_RECEIPT_ITEMS
        || memory_count > MAX_FORGET_RECEIPT_ITEMS as i64;

    let mut source_statement = transaction.prepare(&format!(
        "{AFFECTED}SELECT id FROM affected ORDER BY id LIMIT ?2"
    ))?;
    let sources = source_statement
        .query_map(
            params![source_id.0, MAX_FORGET_RECEIPT_ITEMS as i64],
            |row| Ok(SourceId(row.get(0)?)),
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    push_bounded_unique(
        &mut receipt.suppressed_sources,
        sources,
        &mut receipt.ids_truncated,
    );

    let mut turn_statement = transaction.prepare(&format!(
        "{AFFECTED}SELECT turn_id FROM memory_sources
         WHERE id IN (SELECT id FROM affected) AND turn_id IS NOT NULL
         ORDER BY turn_id LIMIT ?2"
    ))?;
    let turns = turn_statement
        .query_map(
            params![source_id.0, MAX_FORGET_RECEIPT_ITEMS as i64],
            |row| row.get(0),
        )?
        .collect::<rusqlite::Result<Vec<i64>>>()?;
    push_bounded_unique(
        &mut receipt.affected_turn_ids,
        turns,
        &mut receipt.ids_truncated,
    );

    let mut memory_statement = transaction.prepare(&format!(
        "{AFFECTED}SELECT id FROM memory_records
         WHERE source_id IN (SELECT id FROM affected) ORDER BY id LIMIT ?2"
    ))?;
    let memories = memory_statement
        .query_map(
            params![source_id.0, MAX_FORGET_RECEIPT_ITEMS as i64],
            |row| Ok(MemoryId(row.get(0)?)),
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    push_bounded_unique(
        &mut receipt.forgotten_memories,
        memories,
        &mut receipt.ids_truncated,
    );

    transaction.execute(
        &format!(
            "{AFFECTED}INSERT OR IGNORE INTO source_suppressions(
                source_id, suppressed_at_unix_ms, reason
             ) SELECT id, ?2, ?3 FROM affected"
        ),
        params![source_id.0, at, reason],
    )?;
    transaction.execute(
        &format!(
            "{AFFECTED}UPDATE memory_sources
             SET state = 'suppressed', extraction_state = 'suppressed'
             WHERE id IN (SELECT id FROM affected)"
        ),
        [source_id.0],
    )?;
    transaction.execute(
        &format!(
            "{AFFECTED}UPDATE source_parts SET projection_state = 'suppressed'
             WHERE source_id IN (SELECT id FROM affected)"
        ),
        [source_id.0],
    )?;
    transaction.execute(
        &format!(
            "{AFFECTED}UPDATE memory_jobs
             SET status = 'completed', failure_category = 'suppressed',
                 lease_token = lease_token + 1, lease_until_unix_ms = NULL,
                 updated_at_unix_ms = ?2
             WHERE source_id IN (SELECT id FROM affected)
               AND status IN ('queued', 'running')"
        ),
        params![source_id.0, at],
    )?;
    transaction.execute(
        &format!(
            "{AFFECTED}UPDATE memory_records
             SET status = 'forgotten', projection_generation = projection_generation + 1,
                 projection_op = 'delete', projection_lease_token = projection_lease_token + 1,
                 projection_lease_until_unix_ms = NULL, projection_retry_after_unix_ms = 0
             WHERE source_id IN (SELECT id FROM affected) AND status <> 'forgotten'"
        ),
        [source_id.0],
    )?;
    Ok(())
}

fn affected_count(
    transaction: &rusqlite::Transaction<'_>,
    cte: &str,
    table: &str,
    root: SourceId,
) -> Result<usize, rusqlite::Error> {
    let query = match table {
        "memory_sources" => format!("{cte}SELECT COUNT(*) FROM affected"),
        "turns" => format!(
            "{cte}SELECT COUNT(*) FROM memory_sources
             WHERE id IN (SELECT id FROM affected) AND turn_id IS NOT NULL"
        ),
        _ => return Err(rusqlite::Error::InvalidQuery),
    };
    let count: i64 = transaction.query_row(&query, [root.0], |row| row.get(0))?;
    Ok(count.max(0) as usize)
}

fn push_bounded_unique<T: Eq>(
    target: &mut Vec<T>,
    items: impl IntoIterator<Item = T>,
    truncated: &mut bool,
) {
    for item in items {
        if target.contains(&item) {
            continue;
        }
        if target.len() >= MAX_FORGET_RECEIPT_ITEMS {
            *truncated = true;
            break;
        }
        target.push(item);
    }
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

fn scalar_count(connection: &rusqlite::Connection, sql: &str) -> Result<usize, rusqlite::Error> {
    let count: i64 = connection.query_row(sql, [], |row| row.get(0))?;
    Ok(count.max(0) as usize)
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

pub(super) fn parse_record_kind(value: &str) -> Result<MemoryRecordKind, rusqlite::Error> {
    MemoryRecordKind::from_str(value).ok_or(rusqlite::Error::InvalidQuery)
}

pub(super) fn parse_attribution(value: &str) -> Result<MemoryAttribution, rusqlite::Error> {
    MemoryAttribution::from_str(value).ok_or(rusqlite::Error::InvalidQuery)
}

pub(super) fn parse_record_status(value: &str) -> Result<MemoryRecordStatus, rusqlite::Error> {
    MemoryRecordStatus::from_str(value).ok_or(rusqlite::Error::InvalidQuery)
}

pub(super) fn parse_correction_state(value: &str) -> Result<CorrectionState, rusqlite::Error> {
    CorrectionState::from_str(value).ok_or(rusqlite::Error::InvalidQuery)
}

fn parse_source_kind_sql(value: &str) -> Result<SourceKind, rusqlite::Error> {
    parse_source_kind(value)
}

fn parse_source_role_sql(value: &str) -> Result<SourcePartRole, rusqlite::Error> {
    parse_source_role(value)
}
