use std::{
    path::Path,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use arrow_array::{
    Array, FixedSizeListArray, Float32Array, Int64Array, RecordBatch, RecordBatchIterator,
    StringArray,
};
use futures_util::TryStreamExt;
use lance_index::scalar::{
    FullTextSearchQuery,
    inverted::query::{BooleanQuery, FtsQuery, MatchQuery, Occur, collect_query_tokens},
};
use lancedb::{
    Connection, Table,
    arrow::arrow_schema::{ArrowError, DataType, Field, Schema},
    connect,
    index::{Index, scalar::FtsIndexBuilder},
    query::{ExecutableQuery, QueryBase, QueryExecutionOptions},
};
use thiserror::Error;

use crate::{
    clients::local_embeddings::MODEL_FINGERPRINT,
    storage::{
        MemoryAttribution, MemoryId, MemoryRecordKind, ProjectionLease, ProjectionOperation,
    },
};

const TABLE_NAME: &str = "memory_projection_v1";
const SEARCH_LIMIT: usize = 20;
const VECTOR_FIELD: &str = "vector";
const OWNER_MARKER: &str = ".jarvis-memory-projection-owner";
const OWNER_MARKER_VALUE: &str = "jarvis-memory-projection-v1\n";

#[derive(Debug, Error)]
pub(crate) enum IndexError {
    #[error("refusing to manage a non-empty or non-directory unmarked memory index path")]
    UnownedPath,
    #[error("local index I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("local LanceDB operation failed: {0}")]
    Lance(#[from] lancedb::Error),
    #[error("local index returned an invalid Arrow batch: {0}")]
    Arrow(#[from] ArrowError),
    #[error("local index result omitted expected column {0}")]
    MissingColumn(&'static str),
    #[error("local index returned an unexpected column type for {0}")]
    InvalidColumn(&'static str),
    #[error("could not build Lance's configured full-text tokenizer: {0}")]
    InvalidTokenizer(String),
}

pub(crate) struct MemoryIndex {
    _connection: Connection,
    table: Table,
    fingerprint: &'static str,
    pub(crate) was_created: bool,
}

#[derive(Default)]
pub(crate) struct SearchCandidates {
    /// Native reciprocal-rank-fused order; it is deliberately not treated as confidence.
    pub(crate) fused: Vec<MemoryId>,
    /// Best cosine similarity from the independent vector leg.
    pub(crate) cosine: std::collections::HashMap<MemoryId, f32>,
    /// IDs independently returned by the native FTS leg.
    pub(crate) lexical: std::collections::HashSet<MemoryId>,
}

impl MemoryIndex {
    /// The Lance directory is a rebuildable projection. Only a directory with
    /// Jarvis's exact marker may be quarantined; an unmarked path is accepted
    /// only when absent or empty, so configured user data is never claimed.
    pub(crate) async fn open_rebuildable(index_dir: &Path) -> Result<Self, IndexError> {
        let owned = tokio::fs::read_to_string(index_dir.join(OWNER_MARKER))
            .await
            .is_ok_and(|contents| contents == OWNER_MARKER_VALUE);
        if !owned {
            match tokio::fs::symlink_metadata(index_dir).await {
                Ok(metadata) if metadata.file_type().is_dir() => {
                    let mut entries = tokio::fs::read_dir(index_dir).await?;
                    if entries.next_entry().await?.is_some() {
                        return Err(IndexError::UnownedPath);
                    }
                }
                Ok(_) => return Err(IndexError::UnownedPath),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(IndexError::Io(error)),
            }
            // Reserve ownership before Lance writes any files. If initialization
            // fails or the process stops midway, the next open can safely recover.
            reserve_owned_path(index_dir).await?;
        }
        match Self::open(index_dir).await {
            Ok(index) => Ok(index),
            Err(_) => {
                quarantine(index_dir).await?;
                reserve_owned_path(index_dir).await?;
                let index = Self::open(index_dir).await?;
                Ok(index)
            }
        }
    }

    pub(crate) async fn open(index_dir: &Path) -> Result<Self, IndexError> {
        tokio::fs::create_dir_all(index_dir).await?;
        let uri = index_dir.to_string_lossy();
        let connection = connect(uri.as_ref()).execute().await?;
        let (table, table_was_created) = match connection.open_table(TABLE_NAME).execute().await {
            Ok(table) => (table, false),
            Err(_) => {
                let table = connection
                    .create_empty_table(TABLE_NAME, schema())
                    .execute()
                    .await?;
                (table, true)
            }
        };
        let indices = table.list_indices().await?;
        let has_fts = indices.iter().any(|index| {
            index.index_type == lancedb::index::IndexType::FTS
                && index.columns.iter().any(|column| column == "text")
        });
        if !has_fts {
            table
                .create_index(&["text"], Index::FTS(FtsIndexBuilder::default()))
                .execute()
                .await?;
        }
        Ok(Self {
            _connection: connection,
            table,
            fingerprint: MODEL_FINGERPRINT,
            was_created: table_was_created || !has_fts,
        })
    }

    pub(crate) async fn apply(
        &mut self,
        projection: &ProjectionLease,
        vector: Option<Vec<f32>>,
    ) -> Result<(), IndexError> {
        match projection.operation {
            ProjectionOperation::Delete => self.delete(projection.id).await,
            ProjectionOperation::Upsert => {
                let vector = vector.ok_or(IndexError::InvalidColumn(VECTOR_FIELD))?;
                let batch = projection_batch(projection, vector)?;
                let mut merge = self.table.merge_insert(&["memory_id"]);
                merge.when_matched_update_all(None);
                merge.when_not_matched_insert_all();
                let schema = batch.schema();
                let reader =
                    RecordBatchIterator::new(std::iter::once(Ok::<_, ArrowError>(batch)), schema);
                merge.execute(Box::new(reader)).await?;
                Ok(())
            }
        }
    }

    pub(crate) async fn delete(&mut self, id: MemoryId) -> Result<(), IndexError> {
        let predicate = format!("memory_id = {}", id.0);
        self.table.delete(&predicate).await?;
        Ok(())
    }

    pub(crate) async fn search(
        &self,
        query: &str,
        vector: &[f32],
    ) -> Result<SearchCandidates, IndexError> {
        let fingerprint_filter = format!("fingerprint = '{}'", self.fingerprint);
        let mut candidates = SearchCandidates::default();
        let mut vector_order = Vec::new();

        let mut vector_stream = self
            .table
            .query()
            .nearest_to(vector)?
            .distance_type(lancedb::DistanceType::Cosine)
            .only_if(&fingerprint_filter)
            .limit(SEARCH_LIMIT)
            .execute()
            .await?;
        while let Some(batch) = vector_stream.try_next().await? {
            let ids = int64_column(&batch, "memory_id")?;
            let distances = float32_column(&batch, "_distance")?;
            for (id, distance) in ids.iter().zip(distances.iter()) {
                if let (Some(id), Some(distance)) = (id, distance) {
                    candidates.cosine.insert(MemoryId(id), 1.0 - distance);
                    vector_order.push(MemoryId(id));
                }
            }
        }

        let Some(fts_query) = substantive_fts_query(query)? else {
            // LEARNING: when Lance's configured tokenizer removes every query
            // term (for example, a stop-word-only request), vector retrieval is
            // still useful. There is no lexical clause to fuse in this case.
            candidates.fused = vector_order;
            return Ok(candidates);
        };
        let mut lexical_stream = self
            .table
            .query()
            .full_text_search(fts_query.clone())
            .only_if(&fingerprint_filter)
            .limit(SEARCH_LIMIT)
            .execute()
            .await?;
        while let Some(batch) = lexical_stream.try_next().await? {
            candidates.lexical.extend(
                int64_column(&batch, "memory_id")?
                    .iter()
                    .flatten()
                    .map(MemoryId),
            );
        }

        let mut fused_stream = self
            .table
            .query()
            .full_text_search(fts_query)
            .nearest_to(vector)?
            .distance_type(lancedb::DistanceType::Cosine)
            .only_if(&fingerprint_filter)
            .limit(SEARCH_LIMIT)
            // LEARNING: LanceDB owns fusion of the two retrieval legs. The service
            // combines that order with model-specific evidence, never RRF score values.
            .execute_hybrid(QueryExecutionOptions::default())
            .await?;
        while let Some(batch) = fused_stream.try_next().await? {
            candidates.fused.extend(
                int64_column(&batch, "memory_id")?
                    .iter()
                    .flatten()
                    .map(MemoryId),
            );
        }
        Ok(candidates)
    }
}

fn substantive_fts_query(query: &str) -> Result<Option<FullTextSearchQuery>, IndexError> {
    // Use the index builder's exact tokenizer config so query terms receive
    // the same stemming and stop-word filters as the persisted FTS index.
    let mut tokenizer = FtsIndexBuilder::default()
        .build()
        .map_err(|error| IndexError::InvalidTokenizer(error.to_string()))?;
    let tokens = collect_query_tokens(query, &mut tokenizer);
    let mut seen = std::collections::HashSet::new();
    let terms: Vec<_> = tokens
        .into_iter()
        .filter(|term| seen.insert(term.clone()))
        .collect();
    if terms.is_empty() {
        return Ok(None);
    }

    // LEARNING: a single AND MatchQuery in Lance 7 can lose the conjunction
    // when one term has no postings; WAND drops empty postings before counting
    // the required terms. Independent required clauses preserve missing terms
    // through native Boolean intersection and avoid the fallback's OR behavior.
    let must = terms.into_iter().map(|term| {
        let clause = MatchQuery::new(term).with_column(Some("text".to_owned()));
        (Occur::Must, FtsQuery::Match(clause))
    });
    let query = BooleanQuery::new(must);
    Ok(Some(FullTextSearchQuery::new_query(query.into())))
}

async fn reserve_owned_path(index_dir: &Path) -> Result<(), IndexError> {
    tokio::fs::create_dir_all(index_dir).await?;
    tokio::fs::write(index_dir.join(OWNER_MARKER), OWNER_MARKER_VALUE).await?;
    Ok(())
}

async fn quarantine(index_dir: &Path) -> Result<(), IndexError> {
    let parent = index_dir.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "index path has no parent")
    })?;
    let name = index_dir
        .file_name()
        .unwrap_or_else(|| std::ffi::OsStr::new("memory-index"))
        .to_string_lossy();
    let unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    for suffix in 0..100_u32 {
        let backup = parent.join(format!(
            "{name}.unreadable-{unix_ms}-{}-{suffix}",
            std::process::id()
        ));
        if tokio::fs::symlink_metadata(&backup).await.is_ok() {
            continue;
        }
        match tokio::fs::rename(index_dir, &backup).await {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(IndexError::Io(error)),
        }
    }
    Err(IndexError::Io(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "could not allocate a unique LanceDB recovery directory",
    )))
}

fn schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("memory_id", DataType::Int64, false),
        Field::new("source_id", DataType::Int64, false),
        Field::new("source_revision", DataType::Utf8, false),
        Field::new("fingerprint", DataType::Utf8, false),
        Field::new("text", DataType::Utf8, false),
        Field::new("kind", DataType::Utf8, false),
        Field::new("attribution", DataType::Utf8, false),
        Field::new("source_name", DataType::Utf8, true),
        Field::new("created_at_unix_ms", DataType::Int64, false),
        Field::new(
            "vector",
            DataType::FixedSizeList(Arc::new(Field::new("item", DataType::Float32, false)), 384),
            false,
        ),
    ]))
}

fn projection_batch(
    projection: &ProjectionLease,
    vector: Vec<f32>,
) -> Result<RecordBatch, IndexError> {
    if vector.len() != 384 {
        return Err(IndexError::InvalidColumn(VECTOR_FIELD));
    }
    let vector = FixedSizeListArray::try_new(
        Arc::new(Field::new("item", DataType::Float32, false)),
        384,
        Arc::new(Float32Array::from(vector)),
        None,
    )?;
    let schema = schema();
    let text = projection.text.clone();
    let source_name = projection.source_name.clone();
    Ok(RecordBatch::try_new(
        schema,
        vec![
            Arc::new(Int64Array::from(vec![projection.id.0])),
            Arc::new(Int64Array::from(vec![projection.source_id.0])),
            Arc::new(StringArray::from(vec![
                projection.source_revision_sha256.clone(),
            ])),
            Arc::new(StringArray::from(vec![MODEL_FINGERPRINT])),
            Arc::new(StringArray::from(vec![text])),
            Arc::new(StringArray::from(vec![record_kind_name(projection.kind)])),
            Arc::new(StringArray::from(vec![attribution_name(
                projection.attribution,
            )])),
            Arc::new(StringArray::from(vec![source_name])),
            Arc::new(Int64Array::from(vec![projection.created_at_unix_ms])),
            Arc::new(vector),
        ],
    )?)
}

fn int64_column<'a>(
    batch: &'a RecordBatch,
    name: &'static str,
) -> Result<&'a Int64Array, IndexError> {
    batch
        .column_by_name(name)
        .ok_or(IndexError::MissingColumn(name))?
        .as_any()
        .downcast_ref::<Int64Array>()
        .ok_or(IndexError::InvalidColumn(name))
}

fn float32_column<'a>(
    batch: &'a RecordBatch,
    name: &'static str,
) -> Result<&'a Float32Array, IndexError> {
    batch
        .column_by_name(name)
        .ok_or(IndexError::MissingColumn(name))?
        .as_any()
        .downcast_ref::<Float32Array>()
        .ok_or(IndexError::InvalidColumn(name))
}

fn record_kind_name(kind: MemoryRecordKind) -> &'static str {
    match kind {
        MemoryRecordKind::SourceExcerpt => "source_excerpt",
        MemoryRecordKind::ImportedChunk => "imported_chunk",
        MemoryRecordKind::Preference => "preference",
        MemoryRecordKind::Relationship => "relationship",
        MemoryRecordKind::Event => "event",
        MemoryRecordKind::Project => "project",
        MemoryRecordKind::Experience => "experience",
        MemoryRecordKind::Other => "other",
    }
}

fn attribution_name(attribution: MemoryAttribution) -> &'static str {
    match attribution {
        MemoryAttribution::UserStatement => "user_statement",
        MemoryAttribution::AssistantAnswer => "assistant_answer",
        MemoryAttribution::ImportedClaim => "imported_claim",
        MemoryAttribution::ActionResult => "action_result",
        MemoryAttribution::Inference => "inference",
    }
}

#[cfg(test)]
#[path = "../../tests/unit/memory/index_tests.rs"]
mod tests;
