use rusqlite::{TransactionBehavior, params};

use crate::storage::{
    ForgetReceipt, MemoryId, MemoryRepository, SourceId, StorageError, unix_time_ms,
};

const MAX_FORGET_RECEIPT_ITEMS: usize = 128;

impl MemoryRepository {
    pub(crate) async fn forget(&self, id: MemoryId) -> Result<ForgetReceipt, StorageError> {
        let at = unix_time_ms()?;
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
        let at = unix_time_ms()?;
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
pub(super) fn supersede_other_revisions(
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

pub(super) fn reactivate_source_records(
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
