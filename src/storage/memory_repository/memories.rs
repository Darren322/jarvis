use rusqlite::{TransactionBehavior, params, params_from_iter};

use crate::storage::sqlite_vec::{EMBEDDING_DIMENSIONS, vector_blob};
use crate::storage::{
    MemoryId, MemoryKind, MemoryRepository, MemoryStats, MemorySummary, RecalledMemory, SourceId,
    SourceKind, StorageError,
};

const MAX_LIST_ITEMS: usize = 50;
const MAX_SEARCH_RESULTS: usize = 50;
const MAX_EMBEDDING_PAGE: usize = 32;

impl MemoryRepository {
    /// Lists the newest memories, optionally filtered by a case-insensitive
    /// substring.
    pub(crate) async fn list_memories(
        &self,
        query: Option<String>,
        limit: usize,
    ) -> Result<Vec<MemorySummary>, StorageError> {
        let limit = limit.clamp(1, MAX_LIST_ITEMS) as i64;
        let memories = self
            .connection
            .call(move |connection| {
                let mut statement = connection.prepare(
                    "SELECT id, source_id, kind, text FROM memories
                     WHERE ?1 IS NULL OR instr(lower(text), lower(?1)) > 0
                     ORDER BY id DESC LIMIT ?2",
                )?;
                statement
                    .query_map(params![query, limit], |row| {
                        Ok(MemorySummary {
                            id: MemoryId(row.get(0)?),
                            source_id: SourceId(row.get(1)?),
                            kind: MemoryKind::from_sql(&row.get::<_, String>(2)?)?,
                            text: row.get(3)?,
                        })
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .await?;
        Ok(memories)
    }

    pub(crate) async fn stats(
        &self,
        embedding_model: &'static str,
    ) -> Result<MemoryStats, StorageError> {
        let stats = self
            .connection
            .call(move |connection| {
                let count = |sql: &str, params: &[&dyn rusqlite::ToSql]| {
                    connection
                        .query_row(sql, params, |row| row.get::<_, i64>(0))
                        .map(|count| count.max(0) as usize)
                };
                Ok(MemoryStats {
                    memories: count("SELECT COUNT(*) FROM memories", &[])?,
                    pending_embeddings: count(
                        "SELECT COUNT(*) FROM memories WHERE embedding_model IS NOT ?1",
                        &[&embedding_model],
                    )?,
                    conversation_sources: count(
                        "SELECT COUNT(*) FROM sources
                         WHERE kind = 'conversation' AND state = 'active'",
                        &[],
                    )?,
                    import_sources: count(
                        "SELECT COUNT(*) FROM sources WHERE kind = 'import' AND state = 'active'",
                        &[],
                    )?,
                })
            })
            .await?;
        Ok(stats)
    }

    /// Oldest memories without a vector from `embedding_model`.
    pub(crate) async fn pending_embeddings(
        &self,
        embedding_model: &'static str,
        limit: usize,
    ) -> Result<Vec<(MemoryId, String)>, StorageError> {
        let limit = limit.clamp(1, MAX_EMBEDDING_PAGE) as i64;
        let pending = self
            .connection
            .call(move |connection| {
                let mut statement = connection.prepare(
                    "SELECT id, text FROM memories WHERE embedding_model IS NOT ?1
                     ORDER BY id LIMIT ?2",
                )?;
                statement
                    .query_map(params![embedding_model, limit], |row| {
                        Ok((MemoryId(row.get(0)?), row.get(1)?))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .await?;
        Ok(pending)
    }

    /// Stores one vector. Returns `false` when the memory was deleted while it
    /// was being embedded, in which case nothing is written.
    pub(crate) async fn store_embedding(
        &self,
        id: MemoryId,
        embedding_model: &'static str,
        vector: Vec<f32>,
    ) -> Result<bool, StorageError> {
        if vector.len() != EMBEDDING_DIMENSIONS {
            return Err(StorageError::InvalidInput(
                "embedding has the wrong dimensions",
            ));
        }
        let blob = vector_blob(&vector);
        let stored = self
            .connection
            .call(move |connection| {
                let transaction =
                    connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let updated = transaction.execute(
                    "UPDATE memories SET embedding_model = ?2 WHERE id = ?1",
                    params![id.0, embedding_model],
                )?;
                if updated == 1 {
                    transaction.execute("DELETE FROM memory_vectors WHERE rowid = ?1", [id.0])?;
                    transaction.execute(
                        "INSERT INTO memory_vectors (rowid, embedding) VALUES (?1, ?2)",
                        params![id.0, blob],
                    )?;
                }
                transaction.commit()?;
                Ok(updated == 1)
            })
            .await?;
        Ok(stored)
    }

    /// Drops every vector so the idle pass re-embeds all memories.
    pub(crate) async fn reset_embeddings(&self) -> Result<usize, StorageError> {
        let reset = self
            .connection
            .call(|connection| {
                let transaction =
                    connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                transaction.execute("DELETE FROM memory_vectors", [])?;
                let reset = transaction.execute(
                    "UPDATE memories SET embedding_model = NULL WHERE embedding_model IS NOT NULL",
                    [],
                )?;
                transaction.commit()?;
                Ok(reset)
            })
            .await?;
        Ok(reset)
    }

    /// BM25-ranked ids for an FTS5 `MATCH` expression, best first.
    pub(crate) async fn search_keyword(
        &self,
        fts_query: String,
        limit: usize,
    ) -> Result<Vec<MemoryId>, StorageError> {
        let limit = limit.clamp(1, MAX_SEARCH_RESULTS) as i64;
        let ids = self
            .connection
            .call(move |connection| {
                let mut statement = connection.prepare(
                    "SELECT rowid FROM memories_fts WHERE memories_fts MATCH ?1
                     ORDER BY rank LIMIT ?2",
                )?;
                statement
                    .query_map(params![fts_query, limit], |row| Ok(MemoryId(row.get(0)?)))?
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .await?;
        Ok(ids)
    }

    /// Nearest memories by cosine similarity, best first. Vectors from another
    /// embedding model are ignored until they are re-embedded.
    pub(crate) async fn search_vector(
        &self,
        vector: Vec<f32>,
        embedding_model: &'static str,
        limit: usize,
    ) -> Result<Vec<(MemoryId, f32)>, StorageError> {
        if vector.len() != EMBEDDING_DIMENSIONS {
            return Err(StorageError::InvalidInput(
                "embedding has the wrong dimensions",
            ));
        }
        let limit = limit.clamp(1, MAX_SEARCH_RESULTS) as i64;
        let blob = vector_blob(&vector);
        let hits = self
            .connection
            .call(move |connection| {
                let mut statement = connection.prepare(
                    "WITH nearest AS (
                        SELECT rowid, distance FROM memory_vectors
                        WHERE embedding MATCH ?1 AND k = ?2
                     )
                     SELECT n.rowid, n.distance FROM nearest n
                     JOIN memories m ON m.id = n.rowid AND m.embedding_model = ?3
                     ORDER BY n.distance",
                )?;
                statement
                    .query_map(params![blob, limit, embedding_model], |row| {
                        let distance: f64 = row.get(1)?;
                        // Vectors are L2-normalized, so cosine = 1 - d^2 / 2.
                        Ok((
                            MemoryId(row.get(0)?),
                            (1.0 - distance * distance / 2.0) as f32,
                        ))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .await?;
        Ok(hits)
    }

    /// Loads the listed memories that still exist, with source provenance.
    pub(crate) async fn hydrate(
        &self,
        ids: Vec<MemoryId>,
    ) -> Result<Vec<RecalledMemory>, StorageError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let memories = self
            .connection
            .call(move |connection| {
                let placeholders = vec!["?"; ids.len()].join(", ");
                let mut statement = connection.prepare(&format!(
                    "SELECT m.id, m.source_id, s.kind, s.display_name, s.created_at_unix_ms,
                            m.kind, m.text
                     FROM memories m JOIN sources s ON s.id = m.source_id
                     WHERE m.id IN ({placeholders}) AND s.state = 'active'"
                ))?;
                statement
                    .query_map(params_from_iter(ids.iter().map(|id| id.0)), |row| {
                        Ok(RecalledMemory {
                            id: MemoryId(row.get(0)?),
                            source_id: SourceId(row.get(1)?),
                            source_kind: SourceKind::from_sql(&row.get::<_, String>(2)?)?,
                            source_name: row.get(3)?,
                            source_created_at_unix_ms: row.get(4)?,
                            kind: MemoryKind::from_sql(&row.get::<_, String>(5)?)?,
                            text: row.get(6)?,
                        })
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .await?;
        Ok(memories)
    }
}
