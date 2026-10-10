//! A tiny local LanceDB check for the selected embedded hybrid-search backend.
//!
//! The records and vectors are synthetic. This probe validates LanceDB's native FTS + vector
//! query path without a model request, real user content, or a custom fusion implementation.

use std::sync::Arc;

use arrow_array::{FixedSizeListArray, Float32Array, RecordBatch, StringArray};
use futures_util::TryStreamExt;
use lance_index::scalar::FullTextSearchQuery;
use lancedb::rerankers::rrf::RRFReranker;
use lancedb::{
    Connection, Result,
    arrow::arrow_schema::{DataType, Field, Schema},
    connect,
    index::{Index, scalar::FtsIndexBuilder},
    query::{QueryBase, QueryExecutionOptions},
};

/// Runs one temporary-table query to compile and exercise native FTS/vector fusion.
/// The returned IDs must include one result found by each independent search leg.
pub async fn run_local_hybrid_probe() -> Result<Vec<String>> {
    let temp_dir = tempfile::tempdir().map_err(|source| lancedb::Error::Other {
        message: "failed to create a temporary LanceDB probe directory".to_owned(),
        source: Some(Box::new(source)),
    })?;
    let uri = temp_dir.path().to_string_lossy();
    let connection = connect(&uri).execute().await?;
    run_query(&connection).await
}

async fn run_query(connection: &Connection) -> Result<Vec<String>> {
    let table = connection
        .create_table("memory_backend_probe", synthetic_rows()?)
        .execute()
        .await?;
    table
        .create_index(&["text"], Index::FTS(FtsIndexBuilder::default()))
        .execute()
        .await?;

    let mut results = table
        .query()
        .full_text_search(FullTextSearchQuery::new("Kyoto travel".to_owned()))
        .nearest_to(&[1.0_f32, 0.0])?
        .rerank(Arc::new(RRFReranker::default()))
        .limit(2)
        // LEARNING: LanceDB 0.30 executes both native search legs and chooses its default
        // RRFReranker (k = 60) when no custom reranker is attached. RRF scores are rankings,
        // not probabilities or confidence values.
        .execute_hybrid(QueryExecutionOptions::default())
        .await?;

    let mut found = Vec::new();
    while let Some(batch) = results.try_next().await? {
        let id_column = batch
            .column_by_name("id")
            .ok_or_else(|| lancedb::Error::InvalidInput {
                message: "probe result did not include its id column".to_owned(),
            })?;
        let ids = id_column
            .as_any()
            .downcast_ref::<StringArray>()
            .ok_or_else(|| lancedb::Error::InvalidInput {
                message: "probe result id column was not Arrow UTF-8".to_owned(),
            })?;
        found.extend(ids.iter().flatten().map(str::to_owned));
    }
    for expected in ["m-semantic", "m-lexical"] {
        if !found.iter().any(|id| id == expected) {
            return Err(lancedb::Error::InvalidInput {
                message: format!(
                    "native hybrid query omitted {expected}, so both legs were not demonstrated"
                ),
            });
        }
    }
    Ok(found)
}

fn synthetic_rows() -> Result<RecordBatch> {
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Utf8, false),
        Field::new("text", DataType::Utf8, false),
        Field::new(
            "vector",
            DataType::FixedSizeList(Arc::new(Field::new("item", DataType::Float32, false)), 2),
            false,
        ),
    ]));
    let vectors = FixedSizeListArray::try_new(
        Arc::new(Field::new("item", DataType::Float32, false)),
        2,
        Arc::new(Float32Array::from(vec![1.0, 0.0, 0.0, 1.0, -1.0, 0.0])),
        None,
    )?;

    Ok(RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(vec![
                "m-semantic",
                "m-lexical",
                "m-unrelated",
            ])),
            Arc::new(StringArray::from(vec![
                "A Japanese cultural destination is beloved for old wooden temples.",
                "Kyoto travel itinerary with the exact city keyword.",
                "A simple recipe for vegetable soup.",
            ])),
            Arc::new(vectors),
        ],
    )?)
}
