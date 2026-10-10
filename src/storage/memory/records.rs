use std::collections::HashSet;

use rusqlite::params;

use super::corrections::{
    CorrectionDecision, current_correction_is_grounded, find_correction_candidates,
    supersede_corrected_memory,
};
use super::sources::{
    load_source, parse_source_kind, parse_source_role, source_revision_is_active,
};
use crate::storage::{
    CommitDisposition, CorrectionState, HydratedMemory, MemoryAttribution, MemoryEvidence,
    MemoryId, MemoryListFilter, MemoryRecordKind, MemoryRecordStatus, MemoryRepository,
    MemoryStats, MemorySummary, NewMemory, SourceId, SourceKind, SourcePartRole, StorageError,
    unix_time_ms,
};

const MAX_MEMORY_BYTES: usize = 512;
const MAX_EXTRACTION_RECORDS: usize = 32;
const MAX_EVIDENCE_PER_RECORD: usize = 8;
const MAX_EVIDENCE_QUOTE_BYTES: usize = 512;
const MAX_LIST_ITEMS: usize = 50;

impl MemoryRepository {
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
        let now = unix_time_ms()?;
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
}
pub(super) fn commit_new_memories(
    transaction: &rusqlite::Transaction<'_>,
    lease_id: i64,
    source_id: SourceId,
    revision: &str,
    records: Vec<NewMemory>,
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

    let now = unix_time_ms().map_err(|_| rusqlite::Error::InvalidQuery)?;
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

fn scalar_count(connection: &rusqlite::Connection, sql: &str) -> Result<usize, rusqlite::Error> {
    let count: i64 = connection.query_row(sql, [], |row| row.get(0))?;
    Ok(count.max(0) as usize)
}

pub(super) fn parse_record_kind(value: &str) -> Result<MemoryRecordKind, rusqlite::Error> {
    MemoryRecordKind::from_str(value).ok_or(rusqlite::Error::InvalidQuery)
}

pub(super) fn parse_attribution(value: &str) -> Result<MemoryAttribution, rusqlite::Error> {
    MemoryAttribution::from_str(value).ok_or(rusqlite::Error::InvalidQuery)
}

fn parse_record_status(value: &str) -> Result<MemoryRecordStatus, rusqlite::Error> {
    MemoryRecordStatus::from_str(value).ok_or(rusqlite::Error::InvalidQuery)
}

fn parse_correction_state(value: &str) -> Result<CorrectionState, rusqlite::Error> {
    CorrectionState::from_str(value).ok_or(rusqlite::Error::InvalidQuery)
}
