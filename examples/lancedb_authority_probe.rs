//! Isolated LanceDB-authority feasibility probe.
//!
//! Every record, vector, database and path is synthetic and temporary. The SQLite file models a
//! durable outbox only; this example does not read Jarvis configuration, user data, or model files.
//! Its child-process exits exercise two known write boundaries. They do not simulate power loss or
//! interruption during a LanceDB commit.

use std::{
    error::Error,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Arc,
};

use arrow_array::{
    Array, FixedSizeListArray, Float32Array, Int64Array, RecordBatch, RecordBatchIterator,
    StringArray,
};
use futures_util::TryStreamExt;
use lance_index::scalar::FullTextSearchQuery;
use lancedb::{
    Table,
    arrow::arrow_schema::{ArrowError, DataType, Field, Schema},
    connect,
    index::{Index, scalar::FtsIndexBuilder},
    query::{ExecutableQuery, QueryBase, QueryExecutionOptions},
    rerankers::rrf::RRFReranker,
};
use rusqlite::{Connection as SqliteConnection, TransactionBehavior, params};

type ProbeResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

const TABLE_NAME: &str = "authority_probe";
const EXIT_BEFORE_WRITE: i32 = 73;
const EXIT_AFTER_WRITE_BEFORE_ACK: i32 = 74;

#[derive(Clone, Debug)]
struct MemoryRow {
    memory_id: i64,
    generation: i64,
    source_id: i64,
    text: String,
    status: String,
    correction_state: String,
    supersedes_id: Option<i64>,
    vector: [f32; 2],
}

#[derive(Clone, Copy, Debug)]
struct RowState {
    status: &'static str,
    correction_state: &'static str,
    supersedes_id: Option<i64>,
}

#[derive(Debug)]
struct StoredRow {
    memory_id: i64,
    generation: i64,
    text: String,
    status: String,
    correction_state: String,
    supersedes_id: Option<i64>,
}

#[tokio::main]
async fn main() {
    if let Err(error) = dispatch().await {
        eprintln!("LanceDB authority probe failed: {error}");
        std::process::exit(1);
    }
}

async fn dispatch() -> ProbeResult<()> {
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    match args.first().and_then(|arg| arg.to_str()) {
        Some("--child-before-write") => {
            let (lance_dir, outbox_path) = child_paths(&args)?;
            child_before_write(&lance_dir, &outbox_path).await?;
            // Deliberately terminate the child at the known pre-write boundary. The parent
            // expects this injected exit code and then restarts the outbox consumer.
            std::process::exit(EXIT_BEFORE_WRITE);
        }
        Some("--child-after-write-before-ack") => {
            let (lance_dir, outbox_path) = child_paths(&args)?;
            child_after_write_before_ack(&lance_dir, &outbox_path).await?;
            // Lance returned success, but the durable outbox row is intentionally unacknowledged.
            std::process::exit(EXIT_AFTER_WRITE_BEFORE_ACK);
        }
        Some(_) => Err(other("unknown probe argument")),
        None => run_parent().await,
    }
}

fn child_paths(args: &[std::ffi::OsString]) -> ProbeResult<(PathBuf, PathBuf)> {
    if args.len() != 3 {
        return Err(other("child mode requires Lance and outbox paths"));
    }
    Ok((PathBuf::from(&args[1]), PathBuf::from(&args[2])))
}

async fn run_parent() -> ProbeResult<()> {
    let directory = tempfile::tempdir()?;
    let lance_dir = directory.path().join("lance");
    let outbox_path = directory.path().join("outbox.sqlite3");
    let lance_uri = lance_dir.to_string_lossy().into_owned();

    let connection = connect(&lance_uri).execute().await?;
    let table = connection
        .create_table(TABLE_NAME, seed_rows()?)
        .execute()
        .await?;
    table
        .create_index(&["text"], Index::FTS(FtsIndexBuilder::default()))
        .execute()
        .await?;

    // The predicate delete API is exercised on a disposable fixture row. Memory tombstones below
    // stay as rows so an older retry cannot recreate a forgotten record.
    table.delete("memory_id = 900").await?;
    ensure(
        rows_for(&table, "memory_id = 900").await?.is_empty(),
        "predicate delete left the disposable row behind",
    )?;
    println!("predicate_delete=passed");

    let before_write_row = row(
        200,
        1,
        2,
        "A Japanese cultural destination is beloved for old wooden temples.",
        RowState::active(),
        [1.0, 0.0],
    );
    {
        let mut outbox = Outbox::open(&outbox_path)?;
        outbox.enqueue(std::slice::from_ref(&before_write_row))?;
        let failed_write = table.add(mismatched_vector_batch()?).execute().await;
        ensure(
            failed_write.is_err(),
            "deliberate vector-dimension mismatch unexpectedly wrote to LanceDB",
        )?;
        ensure(
            rows_for(&table, "memory_id = 200").await?.is_empty(),
            "failed LanceDB validation partially inserted the pending record",
        )?;
        ensure(
            outbox.pending_count()? == 1,
            "failed LanceDB validation lost the durable pending outbox row",
        )?;
    }
    println!("schema_validation_write_failure_kept_outbox_pending=passed");
    drop(table);
    drop(connection);
    spawn_boundary_child(
        "--child-before-write",
        &lance_dir,
        &outbox_path,
        EXIT_BEFORE_WRITE,
    )?;

    let outbox = Outbox::open(&outbox_path)?;
    ensure(
        outbox.pending_count()? == 1,
        "pre-write exit lost its outbox row",
    )?;
    let connection = connect(&lance_uri).execute().await?;
    let table = connection.open_table(TABLE_NAME).execute().await?;
    ensure(
        rows_for(&table, "memory_id = 200").await?.is_empty(),
        "pre-write child unexpectedly wrote to LanceDB",
    )?;
    replay_pending(&table, &outbox).await?;
    ensure(
        rows_for(&table, "memory_id = 200").await?.len() == 1,
        "restart did not replay the pending outbox row",
    )?;
    drop(outbox);
    drop(table);
    drop(connection);
    println!("pre_write_process_exit_then_restart_replay=passed");

    let after_write_row = row(
        201,
        1,
        3,
        "A synthetic Kyoto food itinerary.",
        RowState::active(),
        [0.0, 1.0],
    );
    {
        let mut outbox = Outbox::open(&outbox_path)?;
        outbox.enqueue(std::slice::from_ref(&after_write_row))?;
    }
    spawn_boundary_child(
        "--child-after-write-before-ack",
        &lance_dir,
        &outbox_path,
        EXIT_AFTER_WRITE_BEFORE_ACK,
    )?;

    let mut outbox = Outbox::open(&outbox_path)?;
    let connection = connect(&lance_uri).execute().await?;
    let table = connection.open_table(TABLE_NAME).execute().await?;
    ensure(
        rows_for(&table, "memory_id = 201").await?.len() == 1,
        "child did not finish the Lance write before its injected exit",
    )?;
    ensure(
        outbox.pending_count()? == 1,
        "post-write exit unexpectedly acknowledged outbox",
    )?;
    replay_pending(&table, &outbox).await?;
    ensure(
        rows_for(&table, "memory_id = 201").await?.len() == 1,
        "idempotent replay duplicated a stable memory ID",
    )?;
    ensure(
        outbox.pending_count()? == 0,
        "successful replay left pending rows",
    )?;

    let updated_row = row(
        201,
        2,
        3,
        "Updated synthetic Kyoto itinerary text.",
        RowState::active(),
        [0.2, 0.8],
    );
    outbox.enqueue(std::slice::from_ref(&updated_row))?;
    replay_pending(&table, &outbox).await?;
    let updated = rows_for(&table, "memory_id = 201").await?;
    ensure(
        updated.len() == 1 && updated[0].generation == 2 && updated[0].text == updated_row.text,
        "higher-generation upsert did not update the stable ID",
    )?;
    outbox.enqueue(std::slice::from_ref(&updated_row))?;
    replay_pending(&table, &outbox).await?;
    ensure(
        rows_for(&table, "memory_id = 201").await?.len() == 1,
        "replaying the same generation was not idempotent",
    )?;
    println!("stable_id_upsert_and_same_generation_replay=passed");

    let hybrid_ids = hybrid_search(&table, "Kyoto travel", &[1.0, 0.0]).await?;
    ensure(
        hybrid_ids.contains(&100) && hybrid_ids.contains(&200),
        "filtered hybrid query did not return its synthetic lexical and vector matches",
    )?;
    println!("filtered_hybrid_search=passed");

    let old_facts = rows_for(&table, "memory_id = 100").await?;
    ensure(
        old_facts.len() == 1,
        "correction fixture did not have exactly one row",
    )?;
    let old_fact = old_facts
        .into_iter()
        .next()
        .ok_or_else(|| other("missing correction fixture"))?;
    ensure(
        old_fact.status == "active",
        "correction fixture was not active",
    )?;
    let superseded = row(
        100,
        old_fact.generation + 1,
        1,
        &old_fact.text,
        RowState {
            status: "superseded",
            correction_state: "none",
            supersedes_id: None,
        },
        [0.0, 1.0],
    );
    let correction = row(
        101,
        1,
        4,
        "I corrected my trip: my destination is Osaka instead of Kyoto.",
        RowState {
            status: "active",
            correction_state: "applied",
            supersedes_id: Some(100),
        },
        [0.0, 1.0],
    );
    outbox.enqueue(&[superseded.clone(), correction.clone()])?;
    replay_pending(&table, &outbox).await?;
    let old_after_correction = rows_for(&table, "memory_id = 100").await?;
    let corrected = rows_for(&table, "memory_id = 101").await?;
    ensure(
        old_after_correction.len() == 1 && old_after_correction[0].status == "superseded",
        "correction did not retire the old synthetic fact",
    )?;
    ensure(
        corrected.len() == 1
            && corrected[0].correction_state == "applied"
            && corrected[0].supersedes_id == Some(100),
        "correction provenance was not preserved",
    )?;

    let version_before_suppression = table.version().await?;
    let tombstone = row(
        101,
        2,
        4,
        &correction.text,
        RowState {
            status: "forgotten",
            correction_state: "applied",
            supersedes_id: Some(100),
        },
        [0.0, 0.0],
    );
    outbox.enqueue(std::slice::from_ref(&tombstone))?;
    replay_pending(&table, &outbox).await?;
    ensure(
        rows_for(&table, "memory_id = 101 AND status = 'active'")
            .await?
            .is_empty(),
        "filtered search exposed a forgotten tombstone",
    )?;
    ensure(
        !hybrid_search(&table, "Osaka", &[0.0, 1.0])
            .await?
            .contains(&101),
        "hybrid search returned a forgotten stable ID",
    )?;

    // A delayed generation-1 outbox event cannot overwrite the generation-2 tombstone. This
    // models a deterministic stale replay and tests the monotonic merge predicate used here.
    outbox.enqueue(std::slice::from_ref(&correction))?;
    replay_pending(&table, &outbox).await?;
    let still_forgotten = rows_for(&table, "memory_id = 101").await?;
    ensure(
        still_forgotten.len() == 1
            && still_forgotten[0].generation == 2
            && still_forgotten[0].status == "forgotten",
        "stale upsert resurrected a tombstoned row",
    )?;
    println!("correction_and_tombstone_stale_retry=passed");

    // Table history can inspect and restore an older version. Restoring a version from before the
    // tombstone does bring the old active row back. The probe then replays the tombstone payload;
    // an acknowledged outbox alone would not retain that payload, so safe production rollback
    // needs a durable suppression/change ledger or an explicit replay source.
    table.checkout(version_before_suppression).await?;
    let checked_out = rows_for(&table, "memory_id = 101").await?;
    ensure(
        checked_out.len() == 1 && checked_out[0].status == "active",
        "historical checkout did not expose the pre-suppression version",
    )?;
    table.restore().await?;
    let restored = rows_for(&table, "memory_id = 101").await?;
    ensure(
        restored.len() == 1 && restored[0].status == "active",
        "restore behavior differed from the checked-out historical state",
    )?;
    outbox.enqueue(std::slice::from_ref(&tombstone))?;
    replay_pending(&table, &outbox).await?;
    ensure(
        rows_for(&table, "memory_id = 101 AND status = 'active'")
            .await?
            .is_empty(),
        "replayed tombstone did not repair the restored historical version",
    )?;
    let version_count = table.list_versions().await?.len();
    println!("table_version_restore_candidate=passed versions={version_count}");
    println!("restore_resurrection_observed=true; tombstone_replay_repaired=true");

    let second_connection = connect(&lance_uri).execute().await?;
    let second_table = second_connection.open_table(TABLE_NAME).execute().await?;
    let (first_view, second_view) = tokio::join!(
        rows_for(&table, "status = 'active'"),
        rows_for(&second_table, "status = 'active'")
    );
    let first_view = first_view?;
    let second_view = second_view?;
    ensure(
        ids(&first_view) == ids(&second_view),
        "concurrent independent handles returned different active rows",
    )?;
    println!("independent_read_handles=passed");

    println!(
        "limitations: process exits were injected at known boundaries; no mid-write or power-loss test; no Jarvis source/evidence/recursive-lineage, correction-job, suppression-lease, stale-recall or multi-source race was exercised; no concurrent writers, physical backup/restore, re-embedding, target-Pi scale/resource or filesystem-durability test was performed"
    );
    Ok(())
}

async fn child_before_write(lance_dir: &Path, outbox_path: &Path) -> ProbeResult<()> {
    let outbox = Outbox::open(outbox_path)?;
    ensure(
        outbox.pending_count()? > 0,
        "pre-write child found no pending outbox row",
    )?;
    let _connection = connect(&lance_dir.to_string_lossy()).execute().await?;
    Ok(())
}

async fn child_after_write_before_ack(lance_dir: &Path, outbox_path: &Path) -> ProbeResult<()> {
    let outbox = Outbox::open(outbox_path)?;
    let connection = connect(&lance_dir.to_string_lossy()).execute().await?;
    let table = connection.open_table(TABLE_NAME).execute().await?;
    let records = outbox.pending()?;
    ensure(
        !records.is_empty(),
        "post-write child found no pending outbox row",
    )?;
    merge_rows(&table, &records).await?;
    Ok(())
}

fn spawn_boundary_child(
    mode: &str,
    lance_dir: &Path,
    outbox_path: &Path,
    expected_exit_code: i32,
) -> ProbeResult<()> {
    let executable = std::env::current_exe()?;
    let status = Command::new(executable)
        .arg(mode)
        .arg(lance_dir)
        .arg(outbox_path)
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    ensure(
        status.code() == Some(expected_exit_code),
        &format!("fault-injection child exited with unexpected status {status}"),
    )
}

async fn replay_pending(table: &Table, outbox: &Outbox) -> ProbeResult<usize> {
    let pending = outbox.pending()?;
    if pending.is_empty() {
        return Ok(0);
    }
    merge_rows(table, &pending).await?;
    let mut acknowledged = 0;
    for row in &pending {
        acknowledged += outbox.acknowledge(row)? as usize;
    }
    Ok(acknowledged)
}

async fn merge_rows(table: &Table, records: &[MemoryRow]) -> ProbeResult<()> {
    if records.is_empty() {
        return Ok(());
    }
    let batch = records_batch(records)?;
    let schema = batch.schema();
    let reader = RecordBatchIterator::new(std::iter::once(Ok::<_, ArrowError>(batch)), schema);
    let mut merge = table.merge_insert(&["memory_id"]);
    merge
        .when_matched_update_all(Some("target.generation < source.generation".to_owned()))
        .when_not_matched_insert_all();
    merge.execute(Box::new(reader)).await?;
    Ok(())
}

async fn rows_for(table: &Table, predicate: &str) -> ProbeResult<Vec<StoredRow>> {
    let mut stream = table.query().only_if(predicate).execute().await?;
    let mut rows = Vec::new();
    while let Some(batch) = stream.try_next().await? {
        let ids = column::<Int64Array>(&batch, "memory_id")?;
        let generations = column::<Int64Array>(&batch, "generation")?;
        let texts = column::<StringArray>(&batch, "text")?;
        let statuses = column::<StringArray>(&batch, "status")?;
        let corrections = column::<StringArray>(&batch, "correction_state")?;
        let supersedes = column::<Int64Array>(&batch, "supersedes_id")?;
        for index in 0..batch.num_rows() {
            rows.push(StoredRow {
                memory_id: ids.value(index),
                generation: generations.value(index),
                text: texts.value(index).to_owned(),
                status: statuses.value(index).to_owned(),
                correction_state: corrections.value(index).to_owned(),
                supersedes_id: (!supersedes.is_null(index)).then(|| supersedes.value(index)),
            });
        }
    }
    Ok(rows)
}

async fn hybrid_search(table: &Table, query: &str, vector: &[f32]) -> ProbeResult<Vec<i64>> {
    let mut stream = table
        .query()
        .full_text_search(FullTextSearchQuery::new(query.to_owned()))
        .nearest_to(vector)?
        .distance_type(lancedb::DistanceType::Cosine)
        .only_if("status = 'active'")
        .rerank(Arc::new(RRFReranker::default()))
        .limit(20)
        .execute_hybrid(QueryExecutionOptions::default())
        .await?;
    let mut found = Vec::new();
    while let Some(batch) = stream.try_next().await? {
        found.extend(column::<Int64Array>(&batch, "memory_id")?.iter().flatten());
    }
    Ok(found)
}

fn records_batch(records: &[MemoryRow]) -> ProbeResult<RecordBatch> {
    let schema = schema();
    let vector_values = records
        .iter()
        .flat_map(|record| record.vector)
        .collect::<Vec<_>>();
    let vectors = FixedSizeListArray::try_new(
        Arc::new(Field::new("item", DataType::Float32, false)),
        2,
        Arc::new(Float32Array::from(vector_values)),
        None,
    )?;
    Ok(RecordBatch::try_new(
        schema,
        vec![
            Arc::new(Int64Array::from(
                records
                    .iter()
                    .map(|record| record.memory_id)
                    .collect::<Vec<_>>(),
            )),
            Arc::new(Int64Array::from(
                records
                    .iter()
                    .map(|record| record.generation)
                    .collect::<Vec<_>>(),
            )),
            Arc::new(Int64Array::from(
                records
                    .iter()
                    .map(|record| record.source_id)
                    .collect::<Vec<_>>(),
            )),
            Arc::new(StringArray::from(
                records
                    .iter()
                    .map(|record| record.text.as_str())
                    .collect::<Vec<_>>(),
            )),
            Arc::new(StringArray::from(
                records
                    .iter()
                    .map(|record| record.status.as_str())
                    .collect::<Vec<_>>(),
            )),
            Arc::new(StringArray::from(
                records
                    .iter()
                    .map(|record| record.correction_state.as_str())
                    .collect::<Vec<_>>(),
            )),
            Arc::new(Int64Array::from(
                records
                    .iter()
                    .map(|record| record.supersedes_id)
                    .collect::<Vec<_>>(),
            )),
            Arc::new(vectors),
        ],
    )?)
}

fn seed_rows() -> ProbeResult<RecordBatch> {
    records_batch(&[
        row(
            100,
            1,
            1,
            "Kyoto travel itinerary with the exact city keyword.",
            RowState::active(),
            [0.0, 1.0],
        ),
        row(
            900,
            1,
            90,
            "Disposable record for testing a predicate delete.",
            RowState::active(),
            [-1.0, 0.0],
        ),
    ])
}

fn mismatched_vector_batch() -> ProbeResult<RecordBatch> {
    let field = Arc::new(Field::new("item", DataType::Float32, false));
    let vectors = FixedSizeListArray::try_new(
        field.clone(),
        3,
        Arc::new(Float32Array::from(vec![0.1, 0.2, 0.3])),
        None,
    )?;
    let schema = Arc::new(Schema::new(vec![Field::new(
        "vector",
        DataType::FixedSizeList(field, 3),
        false,
    )]));
    Ok(RecordBatch::try_new(schema, vec![Arc::new(vectors)])?)
}

fn schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("memory_id", DataType::Int64, false),
        Field::new("generation", DataType::Int64, false),
        Field::new("source_id", DataType::Int64, false),
        Field::new("text", DataType::Utf8, false),
        Field::new("status", DataType::Utf8, false),
        Field::new("correction_state", DataType::Utf8, false),
        Field::new("supersedes_id", DataType::Int64, true),
        Field::new(
            "vector",
            DataType::FixedSizeList(Arc::new(Field::new("item", DataType::Float32, false)), 2),
            false,
        ),
    ]))
}

fn row(
    memory_id: i64,
    generation: i64,
    source_id: i64,
    text: &str,
    state: RowState,
    vector: [f32; 2],
) -> MemoryRow {
    MemoryRow {
        memory_id,
        generation,
        source_id,
        text: text.to_owned(),
        status: state.status.to_owned(),
        correction_state: state.correction_state.to_owned(),
        supersedes_id: state.supersedes_id,
        vector,
    }
}

impl RowState {
    const fn active() -> Self {
        Self {
            status: "active",
            correction_state: "none",
            supersedes_id: None,
        }
    }
}

fn ids(rows: &[StoredRow]) -> Vec<i64> {
    let mut ids = rows.iter().map(|row| row.memory_id).collect::<Vec<_>>();
    ids.sort_unstable();
    ids
}

fn column<'a, T: Array + 'static>(batch: &'a RecordBatch, name: &str) -> ProbeResult<&'a T> {
    let array = batch
        .column_by_name(name)
        .ok_or_else(|| other(format!("result omitted the {name} column")))?;
    array
        .as_any()
        .downcast_ref::<T>()
        .ok_or_else(|| other(format!("result column {name} had an unexpected type")))
}

fn ensure(condition: bool, message: &str) -> ProbeResult<()> {
    if condition {
        Ok(())
    } else {
        Err(other(message))
    }
}

fn other(message: impl Into<String>) -> Box<dyn Error + Send + Sync> {
    Box::new(std::io::Error::other(message.into()))
}

struct Outbox {
    connection: SqliteConnection,
}

impl Outbox {
    fn open(path: &Path) -> ProbeResult<Self> {
        let connection = SqliteConnection::open(path)?;
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS projection_outbox (
                memory_id INTEGER NOT NULL,
                generation INTEGER NOT NULL CHECK (generation > 0),
                source_id INTEGER NOT NULL,
                text TEXT NOT NULL,
                status TEXT NOT NULL CHECK (status IN ('active', 'superseded', 'forgotten')),
                correction_state TEXT NOT NULL CHECK (
                    correction_state IN ('none', 'applied', 'needs_review')
                ),
                supersedes_id INTEGER,
                vector_x REAL NOT NULL,
                vector_y REAL NOT NULL,
                PRIMARY KEY (memory_id, generation)
            );",
        )?;
        Ok(Self { connection })
    }

    fn enqueue(&mut self, rows: &[MemoryRow]) -> ProbeResult<()> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        for row in rows {
            transaction.execute(
                "INSERT INTO projection_outbox (
                    memory_id, generation, source_id, text, status, correction_state,
                    supersedes_id, vector_x, vector_y
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                 ON CONFLICT(memory_id, generation) DO NOTHING",
                params![
                    row.memory_id,
                    row.generation,
                    row.source_id,
                    row.text,
                    row.status,
                    row.correction_state,
                    row.supersedes_id,
                    row.vector[0],
                    row.vector[1],
                ],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    fn pending(&self) -> ProbeResult<Vec<MemoryRow>> {
        let mut statement = self.connection.prepare(
            "SELECT memory_id, generation, source_id, text, status, correction_state,
                    supersedes_id, vector_x, vector_y
             FROM projection_outbox ORDER BY memory_id, generation",
        )?;
        Ok(statement
            .query_map([], |row| {
                Ok(MemoryRow {
                    memory_id: row.get(0)?,
                    generation: row.get(1)?,
                    source_id: row.get(2)?,
                    text: row.get(3)?,
                    status: row.get(4)?,
                    correction_state: row.get(5)?,
                    supersedes_id: row.get(6)?,
                    vector: [row.get::<_, f64>(7)? as f32, row.get::<_, f64>(8)? as f32],
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }

    fn pending_count(&self) -> ProbeResult<usize> {
        let count =
            self.connection
                .query_row("SELECT COUNT(*) FROM projection_outbox", [], |row| {
                    row.get::<_, i64>(0)
                })?;
        Ok(usize::try_from(count)?)
    }

    fn acknowledge(&self, row: &MemoryRow) -> ProbeResult<bool> {
        let changed = self.connection.execute(
            "DELETE FROM projection_outbox WHERE memory_id = ?1 AND generation = ?2",
            params![row.memory_id, row.generation],
        )?;
        Ok(changed == 1)
    }
}
