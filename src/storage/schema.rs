use rusqlite::{Transaction, TransactionBehavior, params};

use super::chunking::{MAX_CHUNK_BYTES, conversation_text, split};

pub(super) const SCHEMA_VERSION: i64 = 6;

const CREATE_SCHEMA_V1: &str = include_str!("migrations/v1.sql");

const MIGRATE_V1_TO_V2: &str = include_str!("migrations/v2.sql");

const MIGRATE_V2_TO_V3: &str = include_str!("migrations/v3.sql");

/// v4 replaces the extraction/projection pipeline (jobs, leases, outbox,
/// LanceDB) with one transactional store: `memories` plus an FTS5 keyword index
/// and a sqlite-vec vector table, both kept in sync by triggers.
const CREATE_V4_TABLES: &str = include_str!("migrations/v4_create_tables.sql");

const DROP_V3_TABLES: &str = include_str!("migrations/v4_drop_v3_tables.sql");

/// v5 adds the facts list: short facts about the user that go into every request.
/// Facts are their own table, never rows in `memories`, so no search returns them.
/// The per-fact byte limit and the list cap are enforced in Rust (see
/// `storage/facts.rs`), not here, so raising one never needs a table rebuild.
/// `AUTOINCREMENT` keeps a removed fact's id from ever naming a later fact.
const MIGRATE_V4_TO_V5: &str = include_str!("migrations/v5.sql");

/// v6 lets Jarvis note facts by itself while idle (fast-memory 05): each turn
/// carries whether that pass has examined it yet. Turns that exist when the
/// database is migrated are marked examined: the pass looks at what is said from
/// now on, and does not start a Gemma run for every past turn on the first idle.
/// (To read the old turns anyway: `UPDATE turns SET facts_examined = 0`.)
/// The partial index keeps "the oldest turn still to examine" cheap on a large
/// archive; the query in `storage/noting.rs` repeats its predicate.
const MIGRATE_V5_TO_V6: &str = include_str!("migrations/v6.sql");

pub(super) enum OpenResult {
    Ready(i64),
    Unsupported(i64),
}

pub(super) fn initialize(
    connection: &mut rusqlite::Connection,
    started_at_unix_ms: i64,
) -> Result<OpenResult, rusqlite::Error> {
    connection.busy_timeout(std::time::Duration::from_secs(1))?;
    connection.execute_batch("PRAGMA foreign_keys = ON;")?;

    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let version: i64 = transaction.query_row("PRAGMA user_version", [], |row| row.get(0))?;

    // LEARNING: each arm falls through the remaining migrations in order, so a
    // database at any older version reaches the current schema in one commit.
    if !(0..=SCHEMA_VERSION).contains(&version) {
        return Ok(OpenResult::Unsupported(version));
    }
    if version < 1 {
        transaction.execute_batch(CREATE_SCHEMA_V1)?;
    }
    if version < 2 {
        transaction.execute_batch(MIGRATE_V1_TO_V2)?;
    }
    if version < 3 {
        transaction.execute_batch(MIGRATE_V2_TO_V3)?;
    }
    if version < 4 {
        migrate_v3_to_v4(&transaction)?;
    }
    if version < 5 {
        transaction.execute_batch(MIGRATE_V4_TO_V5)?;
    }
    if version < 6 {
        transaction.execute_batch(MIGRATE_V5_TO_V6)?;
    }
    let session_id = create_session(&transaction, started_at_unix_ms)?;

    transaction.commit()?;
    Ok(OpenResult::Ready(session_id))
}

/// Moves v3 memory into the v4 tables, keeping source ids so lineage links map
/// one-to-one. Suppressed sources become `forgotten` and keep no memories.
fn migrate_v3_to_v4(transaction: &Transaction<'_>) -> Result<(), rusqlite::Error> {
    transaction.execute_batch(CREATE_V4_TABLES)?;

    transaction.execute(
        "INSERT OR IGNORE INTO sources (
            id, kind, turn_id, path_key, display_name, revision_sha256, state,
            created_at_unix_ms
         )
         SELECT s.id, s.kind,
                CASE WHEN s.kind = 'conversation' THEN s.turn_id END,
                CASE WHEN s.kind = 'import' THEN s.source_key END,
                CASE WHEN s.kind = 'import' THEN COALESCE(s.display_source, s.source_key) END,
                CASE WHEN s.kind = 'import' THEN s.revision_sha256 END,
                CASE
                    WHEN s.state = 'suppressed' OR EXISTS (
                        SELECT 1 FROM source_suppressions ss WHERE ss.source_id = s.id
                    ) THEN 'forgotten'
                    WHEN s.state = 'superseded' THEN 'superseded'
                    ELSE 'active'
                END,
                s.created_at_unix_ms
         FROM memory_sources s
         WHERE s.kind IN ('conversation', 'import')
         ORDER BY s.id",
        [],
    )?;
    transaction.execute(
        "INSERT OR IGNORE INTO source_links (source_id, depends_on_source_id)
         SELECT d.source_id, d.depends_on_source_id FROM source_dependencies d
         WHERE d.source_id IN (SELECT id FROM sources)
           AND d.depends_on_source_id IN (SELECT id FROM sources)",
        [],
    )?;

    // Completed, eligible turns that were never backfilled get a source now.
    transaction.execute(
        "INSERT INTO sources (kind, turn_id, created_at_unix_ms)
         SELECT 'conversation', t.id, t.created_at_unix_ms FROM turns t
         WHERE t.outcome = 'completed' AND t.memory_eligible = 1
           AND NOT EXISTS (SELECT 1 FROM sources s WHERE s.turn_id = t.id)
         ORDER BY t.id",
        [],
    )?;

    // Conversations are re-chunked from the archived turn text.
    let turns = {
        let mut statement = transaction.prepare(
            "SELECT s.id, s.created_at_unix_ms, t.user_text, t.assistant_text
             FROM sources s JOIN turns t ON t.id = s.turn_id
             WHERE s.kind = 'conversation' AND s.state = 'active'
               AND t.assistant_text IS NOT NULL
             ORDER BY s.id",
        )?;
        statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    for (source_id, created_at, user_text, assistant_text) in turns {
        for chunk in split(
            &conversation_text(&user_text, &assistant_text),
            MAX_CHUNK_BYTES,
        ) {
            transaction.execute(
                "INSERT INTO memories (source_id, kind, text, created_at_unix_ms)
                 VALUES (?1, 'excerpt', ?2, ?3)",
                params![source_id, chunk, created_at],
            )?;
        }
    }

    // Imported chunks and extracted facts keep their v3 text.
    transaction.execute(
        "INSERT INTO memories (source_id, kind, text, created_at_unix_ms)
         SELECT r.source_id, 'excerpt', r.text, r.created_at_unix_ms
         FROM memory_records r JOIN sources s ON s.id = r.source_id
         WHERE r.record_kind = 'imported_chunk' AND r.status = 'active'
           AND s.kind = 'import' AND s.state = 'active'
         ORDER BY r.id",
        [],
    )?;
    transaction.execute(
        "INSERT INTO memories (source_id, kind, text, created_at_unix_ms)
         SELECT r.source_id, 'fact', r.text, r.created_at_unix_ms
         FROM memory_records r JOIN sources s ON s.id = r.source_id
         WHERE r.record_kind NOT IN ('source_excerpt', 'imported_chunk')
           AND r.status = 'active' AND s.state = 'active'
         ORDER BY r.id",
        [],
    )?;

    transaction.execute_batch(DROP_V3_TABLES)
}

pub(super) fn create_session(
    transaction: &Transaction<'_>,
    started_at_unix_ms: i64,
) -> Result<i64, rusqlite::Error> {
    transaction.execute(
        "INSERT INTO sessions (started_at_unix_ms) VALUES (?1)",
        [started_at_unix_ms],
    )?;
    Ok(transaction.last_insert_rowid())
}

#[cfg(test)]
#[path = "../../tests/unit/storage/schema_tests.rs"]
mod tests;
