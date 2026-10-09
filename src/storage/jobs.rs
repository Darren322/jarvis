use rusqlite::{OptionalExtension, TransactionBehavior, params};

use super::memory;
use super::{
    BackfillPage, CommitDisposition, JobId, MemoryId, MemoryJobLease, MemoryRepository,
    ProjectionLease, ProjectionOperation, ProjectionOutcome, ProjectionPage, RetryDisposition,
    SafeFailure, SourceId, StorageError,
};

const CLAIM_LEASE_MS: i64 = 60_000;
const MAX_PROJECTION_PAGE: usize = 32;

impl MemoryRepository {
    pub(crate) async fn claim_next_job(&self) -> Result<Option<MemoryJobLease>, StorageError> {
        let now = super::unix_time_ms()?;
        let lease = self
            .connection
            .call(move |connection| {
                let transaction =
                    connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                transaction.execute(
                    "UPDATE memory_jobs
                     SET status = CASE WHEN attempt_count >= 3 THEN 'failed' ELSE 'queued' END,
                         failure_category = CASE WHEN attempt_count >= 3
                             THEN 'attempts_exhausted' ELSE NULL END,
                         lease_until_unix_ms = NULL, updated_at_unix_ms = ?1
                     WHERE status = 'running' AND lease_until_unix_ms <= ?1",
                    [now],
                )?;
                transaction.execute(
                    "UPDATE memory_sources
                     SET extraction_state = CASE WHEN EXISTS (
                             SELECT 1 FROM memory_jobs j
                             WHERE j.source_id = memory_sources.id AND j.status = 'failed'
                         ) THEN 'failed' ELSE 'queued' END
                     WHERE extraction_state = 'running'
                       AND NOT EXISTS (
                           SELECT 1 FROM memory_jobs j
                           WHERE j.source_id = memory_sources.id AND j.status = 'running'
                       )",
                    [],
                )?;

                let running: bool = transaction.query_row(
                    "SELECT EXISTS(SELECT 1 FROM memory_jobs WHERE status = 'running')",
                    [],
                    |row| row.get(0),
                )?;
                if running {
                    transaction.commit()?;
                    return Ok(None);
                }

                let outstanding: i64 = transaction.query_row(
                    "SELECT COUNT(*) FROM memory_jobs WHERE status IN ('queued', 'running')",
                    [],
                    |row| row.get(0),
                )?;
                let slots = (memory::MAX_SOURCE_JOBS - outstanding).clamp(0, 8);
                if slots > 0 {
                    let pending = {
                        let mut statement = transaction.prepare(
                            "SELECT id, revision_sha256 FROM memory_sources
                             WHERE extraction_state = 'pending' AND state = 'active'
                               AND NOT EXISTS (
                                   SELECT 1 FROM source_suppressions ss WHERE ss.source_id = memory_sources.id
                               )
                             ORDER BY id LIMIT ?1",
                        )?;
                        statement
                            .query_map([slots], |row| {
                                Ok((SourceId(row.get(0)?), row.get::<_, String>(1)?))
                            })?
                            .collect::<rusqlite::Result<Vec<_>>>()?
                    };
                    for (source_id, revision) in pending {
                        memory::enqueue_source(&transaction, source_id, &revision, now)?;
                    }
                }

                let queued: Option<(i64, SourceId, String, i64)> = transaction
                    .query_row(
                        "SELECT j.id, j.source_id, j.revision_sha256, j.lease_token
                         FROM memory_jobs j JOIN memory_sources s ON s.id = j.source_id
                         WHERE j.status = 'queued' AND s.state = 'active'
                           AND s.revision_sha256 = j.revision_sha256
                           AND NOT EXISTS (
                               SELECT 1 FROM source_suppressions ss WHERE ss.source_id = s.id
                           )
                         ORDER BY j.created_at_unix_ms, j.id LIMIT 1",
                        [],
                        |row| Ok((row.get(0)?, SourceId(row.get(1)?), row.get(2)?, row.get(3)?)),
                    )
                    .optional()?;
                let Some((job_id, source_id, revision, lease_token)) = queued else {
                    transaction.commit()?;
                    return Ok(None);
                };
                let next_token = lease_token + 1;
                transaction.execute(
                    "UPDATE memory_jobs
                     SET status = 'running', attempt_count = attempt_count + 1,
                         lease_token = ?2, lease_until_unix_ms = ?3,
                         updated_at_unix_ms = ?4
                     WHERE id = ?1 AND status = 'queued'",
                    params![job_id, next_token, now + CLAIM_LEASE_MS, now],
                )?;
                transaction.execute(
                    "UPDATE memory_sources SET extraction_state = 'running' WHERE id = ?1",
                    [source_id.0],
                )?;
                let attempt: u8 = transaction.query_row(
                    "SELECT attempt_count FROM memory_jobs WHERE id = ?1",
                    [job_id],
                    |row| row.get(0),
                )?;
                transaction.commit()?;
                Ok(Some(MemoryJobLease {
                    id: JobId(job_id),
                    source_id,
                    revision_sha256: revision,
                    lease_token: next_token,
                    attempt,
                }))
            })
            .await?;
        Ok(lease)
    }

    pub(crate) async fn commit_extraction(
        &self,
        lease: MemoryJobLease,
        records: Vec<super::NewMemory>,
    ) -> Result<CommitDisposition, StorageError> {
        let now = super::unix_time_ms()?;
        let result = self
            .connection
            .call(move |connection| {
                let transaction =
                    connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let valid_lease: bool = transaction.query_row(
                    "SELECT EXISTS(
                        SELECT 1 FROM memory_jobs
                        WHERE id = ?1 AND status = 'running' AND lease_token = ?2
                          AND source_id = ?3 AND revision_sha256 = ?4
                          AND lease_until_unix_ms > ?5
                    )",
                    params![lease.id.0, lease.lease_token, lease.source_id.0, lease.revision_sha256, now],
                    |row| row.get(0),
                )?;
                if !valid_lease {
                    transaction.commit()?;
                    return Ok(CommitDisposition::LeaseLost);
                }
                let source_active = memory::source_revision_is_active(
                    &transaction,
                    lease.source_id,
                    &lease.revision_sha256,
                )?;
                if !source_active {
                    transaction.execute(
                        "UPDATE memory_jobs SET status = 'completed', failure_category = 'source_changed',
                             lease_until_unix_ms = NULL, updated_at_unix_ms = ?2 WHERE id = ?1",
                        params![lease.id.0, now],
                    )?;
                    transaction.commit()?;
                    return Ok(CommitDisposition::Suppressed);
                }
                let disposition = memory::commit_new_memories(
                    &transaction,
                    lease.id.0,
                    lease.source_id,
                    &lease.revision_sha256,
                    records,
                )?;
                match disposition {
                    CommitDisposition::InvalidEvidence => {
                        transaction.execute(
                            "UPDATE memory_jobs SET status = 'failed', failure_category = 'invalid_extraction',
                                 lease_until_unix_ms = NULL, updated_at_unix_ms = ?2 WHERE id = ?1",
                            params![lease.id.0, now],
                        )?;
                        transaction.execute(
                            "UPDATE memory_sources SET extraction_state = 'failed' WHERE id = ?1",
                            [lease.source_id.0],
                        )?;
                    }
                    _ => {
                        transaction.execute(
                            "UPDATE memory_jobs SET status = 'completed', failure_category = NULL,
                                 lease_until_unix_ms = NULL, updated_at_unix_ms = ?2 WHERE id = ?1",
                            params![lease.id.0, now],
                        )?;
                        transaction.execute(
                            "UPDATE memory_sources SET extraction_state = 'complete' WHERE id = ?1",
                            [lease.source_id.0],
                        )?;
                    }
                }
                transaction.commit()?;
                Ok(disposition)
            })
            .await?;
        Ok(result)
    }

    pub(crate) async fn fail_job(
        &self,
        lease: MemoryJobLease,
        failure: SafeFailure,
    ) -> Result<(), StorageError> {
        let now = super::unix_time_ms()?;
        self.connection
            .call(move |connection| {
                let transaction =
                    connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let changed = transaction.execute(
                    "UPDATE memory_jobs
                     SET status = 'failed', failure_category = ?3,
                         lease_until_unix_ms = NULL, updated_at_unix_ms = ?4
                     WHERE id = ?1 AND status = 'running' AND lease_token = ?2",
                    params![lease.id.0, lease.lease_token, failure.as_str(), now],
                )?;
                if changed != 0 {
                    transaction.execute(
                        "UPDATE memory_sources SET extraction_state = 'failed' WHERE id = ?1",
                        [lease.source_id.0],
                    )?;
                }
                transaction.commit()?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    /// Returns a canceled idle lease to the durable queue. An admitted model
    /// call consumes its attempt even when its future is dropped.
    pub(crate) async fn release_job(
        &self,
        lease: MemoryJobLease,
        attempted: bool,
    ) -> Result<(), StorageError> {
        let now = super::unix_time_ms()?;
        self.connection
            .call(move |connection| {
                let transaction =
                    connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let source_active: bool = transaction.query_row(
                    "SELECT EXISTS(
                        SELECT 1 FROM memory_sources s
                        WHERE s.id = ?1 AND s.revision_sha256 = ?2 AND s.state = 'active'
                          AND NOT EXISTS (
                              SELECT 1 FROM source_suppressions ss WHERE ss.source_id = s.id
                          )
                    )",
                    params![lease.source_id.0, lease.revision_sha256],
                    |row| row.get(0),
                )?;
                let changed = transaction.execute(
                    "UPDATE memory_jobs
                     SET status = CASE
                             WHEN ?5 = 0 THEN 'completed'
                             WHEN ?6 = 1 AND attempt_count >= 3 THEN 'failed'
                             ELSE 'queued'
                         END,
                         failure_category = CASE
                             WHEN ?5 = 0 THEN 'source_changed'
                             WHEN ?6 = 1 AND attempt_count >= 3 THEN 'attempts_exhausted'
                             ELSE NULL
                         END,
                         attempt_count = CASE WHEN ?6 = 1 THEN attempt_count
                                              ELSE MAX(0, attempt_count - 1) END,
                         lease_token = lease_token + 1, lease_until_unix_ms = NULL,
                         updated_at_unix_ms = ?7
                     WHERE id = ?1 AND status = 'running' AND lease_token = ?2
                       AND source_id = ?3 AND revision_sha256 = ?4
                       AND lease_until_unix_ms > ?7",
                    params![
                        lease.id.0,
                        lease.lease_token,
                        lease.source_id.0,
                        lease.revision_sha256,
                        source_active,
                        attempted,
                        now,
                    ],
                )?;
                if changed != 0 && source_active {
                    transaction.execute(
                        "UPDATE memory_sources
                         SET extraction_state = CASE
                             WHEN EXISTS (
                                 SELECT 1 FROM memory_jobs j
                                 WHERE j.id = ?2 AND j.status = 'failed'
                             ) THEN 'failed' ELSE 'queued' END
                         WHERE id = ?1 AND revision_sha256 = ?3 AND state = 'active'
                           AND extraction_state = 'running'
                           AND NOT EXISTS (
                               SELECT 1 FROM source_suppressions ss WHERE ss.source_id = ?1
                           )",
                        params![lease.source_id.0, lease.id.0, lease.revision_sha256],
                    )?;
                }
                transaction.commit()?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    pub(crate) async fn retry_failed(&self, id: JobId) -> Result<RetryDisposition, StorageError> {
        let now = super::unix_time_ms()?;
        let result = self
            .connection
            .call(move |connection| {
                let transaction =
                    connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let job: Option<(SourceId, i64, String)> = transaction
                    .query_row(
                        "SELECT j.source_id, j.attempt_count, s.state
                         FROM memory_jobs j JOIN memory_sources s ON s.id = j.source_id
                         WHERE j.id = ?1 AND j.status = 'failed'",
                        [id.0],
                        |row| Ok((SourceId(row.get(0)?), row.get(1)?, row.get(2)?)),
                    )
                    .optional()?;
                let Some((source_id, attempts, source_state)) = job else {
                    transaction.commit()?;
                    return Ok(RetryDisposition::NotRetryable);
                };
                if attempts >= 3 {
                    transaction.commit()?;
                    return Ok(RetryDisposition::AttemptsExhausted);
                }
                if source_state != "active"
                    || memory::source_is_suppressed(&transaction, source_id)?
                {
                    transaction.commit()?;
                    return Ok(RetryDisposition::NotRetryable);
                }
                let outstanding: i64 = transaction.query_row(
                    "SELECT COUNT(*) FROM memory_jobs WHERE status IN ('queued', 'running')",
                    [],
                    |row| row.get(0),
                )?;
                if outstanding >= memory::MAX_SOURCE_JOBS {
                    transaction.execute(
                        "UPDATE memory_sources SET extraction_state = 'pending' WHERE id = ?1",
                        [source_id.0],
                    )?;
                    transaction.commit()?;
                    return Ok(RetryDisposition::Pending);
                }
                transaction.execute(
                    "UPDATE memory_jobs SET status = 'queued', failure_category = NULL,
                         updated_at_unix_ms = ?2 WHERE id = ?1",
                    params![id.0, now],
                )?;
                transaction.execute(
                    "UPDATE memory_sources SET extraction_state = 'queued' WHERE id = ?1",
                    [source_id.0],
                )?;
                transaction.commit()?;
                Ok(RetryDisposition::Requeued)
            })
            .await?;
        Ok(result)
    }

    pub(crate) async fn backfill_projection_page(
        &self,
        limit: usize,
    ) -> Result<BackfillPage, StorageError> {
        let limit = limit.clamp(1, MAX_PROJECTION_PAGE) as i64;
        let page = self
            .connection
            .call(move |connection| {
                let transaction =
                    connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let cursor: i64 = transaction.query_row(
                    "SELECT integer_value FROM memory_state WHERE key = 'backfill_cursor_turn_id'",
                    [],
                    |row| row.get(0),
                )?;
                let rows = {
                    let mut statement = transaction.prepare(
                        "SELECT id, created_at_unix_ms, user_text, assistant_text,
                                outcome, memory_eligible
                         FROM turns WHERE id > ?1 ORDER BY id LIMIT ?2",
                    )?;
                    statement
                        .query_map(params![cursor, limit], |row| {
                            Ok((
                                row.get::<_, i64>(0)?,
                                row.get::<_, i64>(1)?,
                                row.get::<_, String>(2)?,
                                row.get::<_, Option<String>>(3)?,
                                row.get::<_, String>(4)?,
                                row.get::<_, bool>(5)?,
                            ))
                        })?
                        .collect::<rusqlite::Result<Vec<_>>>()?
                };
                let mut created_sources = 0;
                let mut next_cursor = cursor;
                for (turn_id, created_at_unix_ms, user, assistant, outcome, eligible) in &rows {
                    next_cursor = *turn_id;
                    if outcome != "completed" || !eligible {
                        continue;
                    }
                    let Some(assistant) = assistant.as_deref() else {
                        continue;
                    };
                    let source_key = format!("turn:{turn_id}");
                    let revision = memory::conversation_hash(user, assistant);
                    let exists: bool = transaction.query_row(
                        "SELECT EXISTS(
                            SELECT 1 FROM memory_sources
                            WHERE kind = 'conversation' AND source_key = ?1
                              AND revision_sha256 = ?2
                        )",
                        params![source_key, revision],
                        |row| row.get(0),
                    )?;
                    if exists {
                        continue;
                    }
                    let _ = memory::insert_conversation_source(
                        &transaction,
                        *turn_id,
                        user,
                        assistant,
                        *created_at_unix_ms,
                        super::MemoryEligibility::ArchiveOnly,
                        &[],
                    )?;
                    created_sources += 1;
                }
                if next_cursor != cursor {
                    transaction.execute(
                        "UPDATE memory_state SET integer_value = ?1
                         WHERE key = 'backfill_cursor_turn_id'",
                        [next_cursor],
                    )?;
                }
                let more: bool = transaction.query_row(
                    "SELECT EXISTS(SELECT 1 FROM turns WHERE id > ?1)",
                    [next_cursor],
                    |row| row.get(0),
                )?;
                let page = BackfillPage {
                    examined_turns: rows.len(),
                    created_sources,
                    cursor_turn_id: next_cursor,
                    complete: !more,
                };
                transaction.commit()?;
                Ok(page)
            })
            .await?;
        Ok(page)
    }

    pub(crate) async fn claim_projection_page(
        &self,
        limit: usize,
    ) -> Result<ProjectionPage, StorageError> {
        let limit = limit.clamp(1, MAX_PROJECTION_PAGE) as i64;
        let now = super::unix_time_ms()?;
        let page = self
            .connection
            .call(move |connection| {
                let transaction =
                    connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let rows = {
                    let mut statement = transaction.prepare(
                        "SELECT m.id, m.source_id, s.revision_sha256, s.kind, s.display_source,
                                m.created_at_unix_ms, m.projection_generation, m.projection_op,
                                m.text, m.record_kind, m.attribution, m.projection_lease_token
                         FROM memory_records m
                         JOIN memory_sources s ON s.id = m.source_id
                         WHERE m.projected_generation < m.projection_generation
                           AND m.projection_retry_after_unix_ms <= ?1
                           AND (m.projection_lease_until_unix_ms IS NULL
                                OR m.projection_lease_until_unix_ms <= ?1)
                           AND (m.projection_op = 'delete'
                                OR (m.status = 'active' AND s.state = 'active'
                                    AND NOT EXISTS (
                                        SELECT 1 FROM source_suppressions ss WHERE ss.source_id = s.id
                                    )))
                         ORDER BY m.id LIMIT ?2",
                    )?;
                    statement
                        .query_map(params![now, limit], |row| {
                            Ok((
                                MemoryId(row.get(0)?),
                                SourceId(row.get(1)?),
                                row.get::<_, String>(2)?,
                                row.get::<_, String>(3)?,
                                row.get::<_, Option<String>>(4)?,
                                row.get::<_, i64>(5)?,
                                row.get::<_, i64>(6)?,
                                row.get::<_, String>(7)?,
                                row.get::<_, String>(8)?,
                                row.get::<_, String>(9)?,
                                row.get::<_, String>(10)?,
                                row.get::<_, i64>(11)?,
                            ))
                        })?
                        .collect::<rusqlite::Result<Vec<_>>>()?
                };
                let mut items = Vec::with_capacity(rows.len());
                for (
                    id,
                    source_id,
                    source_revision_sha256,
                    source_kind,
                    source_name,
                    created_at_unix_ms,
                    generation,
                    operation,
                    text,
                    kind,
                    attribution,
                    old_token,
                ) in rows
                {
                    let token = old_token + 1;
                    transaction.execute(
                        "UPDATE memory_records
                         SET projection_lease_token = ?2,
                             projection_lease_until_unix_ms = ?3
                         WHERE id = ?1 AND projection_generation = ?4
                           AND projected_generation < projection_generation",
                        params![id.0, token, now + CLAIM_LEASE_MS, generation],
                    )?;
                    items.push(ProjectionLease {
                        id,
                        source_id,
                        source_revision_sha256,
                        source_kind: memory::parse_source_kind(&source_kind)?,
                        source_name,
                        created_at_unix_ms,
                        generation,
                        lease_token: token,
                        operation: if operation == "delete" {
                            ProjectionOperation::Delete
                        } else {
                            ProjectionOperation::Upsert
                        },
                        text,
                        kind: memory::parse_record_kind(&kind)?,
                        attribution: memory::parse_attribution(&attribution)?,
                    });
                }
                transaction.commit()?;
                Ok(ProjectionPage { items })
            })
            .await?;
        Ok(page)
    }

    pub(crate) async fn finish_projection(
        &self,
        lease: ProjectionLease,
        outcome: ProjectionOutcome,
    ) -> Result<bool, StorageError> {
        let now = super::unix_time_ms()?;
        let applied = self
            .connection
            .call(move |connection| {
                let transaction =
                    connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let current: Option<i64> = transaction
                    .query_row(
                        "SELECT projection_generation FROM memory_records
                         WHERE id = ?1 AND projection_lease_token = ?2
                           AND projection_generation = ?3",
                        params![lease.id.0, lease.lease_token, lease.generation],
                        |row| row.get(0),
                    )
                    .optional()?;
                if current.is_none() {
                    transaction.commit()?;
                    return Ok(false);
                }
                let success = outcome == ProjectionOutcome::Applied;
                if success {
                    transaction.execute(
                        "UPDATE memory_records SET projected_generation = ?2,
                             projection_lease_until_unix_ms = NULL, projection_failures = 0,
                             projection_retry_after_unix_ms = 0
                         WHERE id = ?1 AND projection_generation = ?2
                           AND projection_lease_token = ?3",
                        params![lease.id.0, lease.generation, lease.lease_token],
                    )?;
                } else {
                    transaction.execute(
                        "UPDATE memory_records SET projection_lease_until_unix_ms = NULL,
                             projection_failures = projection_failures + 1,
                             projection_retry_after_unix_ms = ?2 +
                                 MIN(600000, 30000 * (1 << MIN(projection_failures, 4)))
                         WHERE id = ?1 AND projection_generation = ?3
                           AND projection_lease_token = ?4",
                        params![lease.id.0, now, lease.generation, lease.lease_token],
                    )?;
                }
                transaction.commit()?;
                Ok(success)
            })
            .await?;
        Ok(applied)
    }
}
