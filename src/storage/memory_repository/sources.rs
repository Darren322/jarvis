use std::collections::HashSet;

use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};
use sha2::{Digest, Sha256};

use crate::storage::chunking::{MAX_CHUNK_BYTES, conversation_text, split};
use crate::storage::time::unix_time_ms;
use crate::storage::{
    ForgetReceipt, ImportReceipt, ImportState, MemoryId, MemoryRepository, MemorySaveState,
    SourceId, StorageError,
};

const MAX_IMPORT_BYTES: usize = 1024 * 1024;
pub(in crate::storage) const MAX_RECALLED_SOURCES: usize = 32;

/// Every source reachable from `?1` through dependency links, including itself.
const DESCENDANTS: &str = "WITH RECURSIVE affected(id) AS (
    SELECT ?1
    UNION
    SELECT l.source_id FROM source_links l JOIN affected a ON l.depends_on_source_id = a.id
) ";

impl MemoryRepository {
    /// Saves an imported file as chunked memories. Re-importing the same path
    /// with new content supersedes the previous revision; a revision that was
    /// forgotten stays forgotten.
    pub(crate) async fn import_source(
        &self,
        path_key: String,
        display_name: String,
        text: String,
    ) -> Result<ImportReceipt, StorageError> {
        if text.len() > MAX_IMPORT_BYTES {
            return Err(StorageError::SourceTooLarge(MAX_IMPORT_BYTES));
        }
        if path_key.trim().is_empty() || display_name.trim().is_empty() {
            return Err(StorageError::InvalidInput("import path is empty"));
        }
        let chunks = split(&text, MAX_CHUNK_BYTES);
        if chunks.is_empty() {
            return Err(StorageError::InvalidInput("import is empty"));
        }

        let revision = format!("{:x}", Sha256::digest(text.as_bytes()));
        let now = unix_time_ms()?;
        let receipt = self
            .connection
            .call(move |connection| {
                let transaction =
                    connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let existing: Option<(i64, String)> = transaction
                    .query_row(
                        "SELECT id, state FROM sources
                         WHERE kind = 'import' AND path_key = ?1 AND revision_sha256 = ?2",
                        params![path_key, revision],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .optional()?;

                let (source_id, state) = match existing {
                    Some((id, state)) if state == "forgotten" => {
                        transaction.commit()?;
                        return Ok(ImportReceipt {
                            source_id: SourceId(id),
                            state: ImportState::Forgotten,
                            chunks: 0,
                        });
                    }
                    Some((id, state)) if state == "active" => {
                        transaction.commit()?;
                        return Ok(ImportReceipt {
                            source_id: SourceId(id),
                            state: ImportState::AlreadyCurrent,
                            chunks: 0,
                        });
                    }
                    Some((id, _superseded)) => {
                        transaction
                            .execute("UPDATE sources SET state = 'active' WHERE id = ?1", [id])?;
                        (SourceId(id), ImportState::Reactivated)
                    }
                    None => {
                        transaction.execute(
                            "INSERT INTO sources (
                                kind, path_key, display_name, revision_sha256, created_at_unix_ms
                             ) VALUES ('import', ?1, ?2, ?3, ?4)",
                            params![path_key, display_name, revision, now],
                        )?;
                        (
                            SourceId(transaction.last_insert_rowid()),
                            ImportState::Created,
                        )
                    }
                };

                let others = {
                    let mut statement = transaction.prepare(
                        "SELECT id FROM sources
                         WHERE kind = 'import' AND path_key = ?1 AND id <> ?2 AND state = 'active'",
                    )?;
                    statement
                        .query_map(params![path_key, source_id.0], |row| {
                            Ok(SourceId(row.get(0)?))
                        })?
                        .collect::<rusqlite::Result<Vec<_>>>()?
                };
                for other in others {
                    retire_lineage(&transaction, other, "superseded")?;
                }

                let chunk_count = chunks.len();
                insert_memories(&transaction, source_id, &chunks, now)?;
                transaction.commit()?;
                Ok(ImportReceipt {
                    source_id,
                    state,
                    chunks: chunk_count,
                })
            })
            .await?;
        Ok(receipt)
    }

    /// Forgets the source a memory belongs to, and everything built from it.
    pub(crate) async fn forget_memory(&self, id: MemoryId) -> Result<ForgetReceipt, StorageError> {
        let receipt = self
            .connection
            .call(move |connection| {
                let transaction =
                    connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let source: Option<i64> = transaction
                    .query_row(
                        "SELECT source_id FROM memories WHERE id = ?1",
                        [id.0],
                        |row| row.get(0),
                    )
                    .optional()?;
                let receipt = match source {
                    Some(source_id) => {
                        retire_lineage(&transaction, SourceId(source_id), "forgotten")?
                    }
                    None => ForgetReceipt::default(),
                };
                transaction.commit()?;
                Ok(receipt)
            })
            .await?;
        Ok(receipt)
    }

    /// Forgets a source and every source that recalled it, transitively.
    pub(crate) async fn forget_source(
        &self,
        source_id: SourceId,
    ) -> Result<ForgetReceipt, StorageError> {
        let receipt = self
            .connection
            .call(move |connection| {
                let transaction =
                    connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let receipt = retire_lineage(&transaction, source_id, "forgotten")?;
                transaction.commit()?;
                Ok(receipt)
            })
            .await?;
        Ok(receipt)
    }
}

/// Records a completed turn as a conversation source in the archive's
/// transaction. If any recalled dependency was forgotten meanwhile, the turn
/// is archived but its source starts out forgotten.
pub(in crate::storage) fn insert_conversation_source(
    transaction: &Transaction<'_>,
    turn_id: i64,
    user_text: &str,
    assistant_text: &str,
    created_at_unix_ms: i64,
    recalled_source_ids: &[SourceId],
) -> Result<(SourceId, MemorySaveState), rusqlite::Error> {
    transaction.execute(
        "INSERT INTO sources (kind, turn_id, created_at_unix_ms) VALUES ('conversation', ?1, ?2)",
        params![turn_id, created_at_unix_ms],
    )?;
    let source_id = SourceId(transaction.last_insert_rowid());

    let mut dependencies_valid = recalled_source_ids.len() <= MAX_RECALLED_SOURCES;
    let mut seen = HashSet::new();
    for dependency in recalled_source_ids.iter().take(MAX_RECALLED_SOURCES) {
        if !seen.insert(*dependency) {
            continue;
        }
        let state: Option<String> = transaction
            .query_row(
                "SELECT state FROM sources WHERE id = ?1 AND id < ?2",
                params![dependency.0, source_id.0],
                |row| row.get(0),
            )
            .optional()?;
        match state {
            Some(state) => {
                transaction.execute(
                    "INSERT INTO source_links (source_id, depends_on_source_id) VALUES (?1, ?2)",
                    params![source_id.0, dependency.0],
                )?;
                dependencies_valid &= state == "active";
            }
            None => dependencies_valid = false,
        }
    }

    if !dependencies_valid {
        transaction.execute(
            "UPDATE sources SET state = 'forgotten' WHERE id = ?1",
            [source_id.0],
        )?;
        return Ok((source_id, MemorySaveState::Suppressed));
    }
    let chunks = split(
        &conversation_text(user_text, assistant_text),
        MAX_CHUNK_BYTES,
    );
    insert_memories(transaction, source_id, &chunks, created_at_unix_ms)?;
    Ok((source_id, MemorySaveState::Saved))
}

fn insert_memories(
    transaction: &Transaction<'_>,
    source_id: SourceId,
    chunks: &[String],
    created_at_unix_ms: i64,
) -> Result<(), rusqlite::Error> {
    let mut statement = transaction.prepare(
        "INSERT INTO memories (source_id, kind, text, created_at_unix_ms)
         VALUES (?1, 'excerpt', ?2, ?3)",
    )?;
    for chunk in chunks {
        statement.execute(params![source_id.0, chunk, created_at_unix_ms])?;
    }
    Ok(())
}

/// Marks `root` and its descendants with `state` and deletes their memories;
/// the delete trigger removes the matching keyword and vector rows.
fn retire_lineage(
    transaction: &Transaction<'_>,
    root: SourceId,
    state: &str,
) -> Result<ForgetReceipt, rusqlite::Error> {
    let count = |sql: &str| -> rusqlite::Result<usize> {
        let count: i64 =
            transaction.query_row(&format!("{DESCENDANTS}{sql}"), [root.0], |row| row.get(0))?;
        Ok(count.max(0) as usize)
    };
    let forgotten_memories =
        count("SELECT COUNT(*) FROM memories WHERE source_id IN (SELECT id FROM affected)")?;
    let affected_turns = count(
        "SELECT COUNT(*) FROM sources
         WHERE id IN (SELECT id FROM affected) AND turn_id IS NOT NULL",
    )?;
    transaction.execute(
        &format!("{DESCENDANTS}DELETE FROM memories WHERE source_id IN (SELECT id FROM affected)"),
        [root.0],
    )?;
    // Superseding never overwrites a forgotten source.
    let forgotten_sources = transaction.execute(
        &format!(
            "{DESCENDANTS}UPDATE sources SET state = ?2
             WHERE id IN (SELECT id FROM affected) AND state <> 'forgotten' AND state <> ?2"
        ),
        params![root.0, state],
    )?;
    Ok(ForgetReceipt {
        forgotten_memories,
        forgotten_sources,
        affected_turns,
    })
}
