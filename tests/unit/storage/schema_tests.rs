use rusqlite::Connection;

use super::{
    CREATE_SCHEMA_V1, MIGRATE_V1_TO_V2, MIGRATE_V2_TO_V3, OpenResult, SCHEMA_VERSION, initialize,
};

/// A v3 database with one of each case the migration must handle.
fn v3_database() -> Connection {
    crate::storage::sqlite_vec::register();
    let connection = Connection::open_in_memory().expect("in-memory database opens");
    connection
        .execute_batch(&format!(
            "PRAGMA foreign_keys = ON; {CREATE_SCHEMA_V1} {MIGRATE_V1_TO_V2} {MIGRATE_V2_TO_V3}"
        ))
        .expect("v3 schema builds");
    connection
        .execute_batch(
            "INSERT INTO sessions (id, started_at_unix_ms) VALUES (1, 100);
             INSERT INTO turns (id, session_id, created_at_unix_ms, user_text, assistant_text, outcome)
             VALUES
                 (1, 1, 101, 'My cat is called Miso.', 'Noted, master.', 'completed'),
                 (2, 1, 102, 'My PIN is 4321.', 'Noted.', 'completed'),
                 (3, 1, 103, 'I enjoy hiking in the alps.', 'Lovely.', 'completed'),
                 (4, 1, 104, 'Status?', NULL, 'timed_out');

             -- Turn 1 is active; turn 2 was forgotten and depended on turn 1;
             -- turn 3 was never backfilled; source 3 is an imported file.
             INSERT INTO memory_sources
                 (id, kind, source_key, revision_sha256, turn_id, created_at_unix_ms, state)
             VALUES
                 (1, 'conversation', 'turn:1', 'r1', 1, 101, 'active'),
                 (2, 'conversation', 'turn:2', 'r2', 2, 102, 'suppressed');
             INSERT INTO memory_sources
                 (id, kind, source_key, revision_sha256, display_source, created_at_unix_ms, state)
             VALUES (3, 'import', '/notes/garden.md', 'r3', 'garden.md', 105, 'active');
             INSERT INTO source_suppressions (source_id, suppressed_at_unix_ms, reason)
             VALUES (2, 106, 'forgotten_source');
             INSERT INTO source_dependencies (source_id, depends_on_source_id) VALUES (2, 1);

             INSERT INTO memory_records
                 (source_id, record_key, text, record_kind, attribution, created_at_unix_ms)
             VALUES
                 (1, 'fact:1', 'The user''s cat is named Miso.', 'preference', 'user_statement', 107),
                 (1, 'chunk:1', 'My cat is called Miso.', 'source_excerpt', 'user_statement', 107),
                 (2, 'fact:2', 'The user''s PIN is 4321.', 'other', 'user_statement', 107),
                 (3, 'chunk:3', 'Tomatoes need full sun.', 'imported_chunk', 'imported_claim', 108);",
        )
        .expect("v3 fixture rows insert");
    connection
}

fn count(connection: &Connection, sql: &str) -> i64 {
    connection
        .query_row(sql, [], |row| row.get(0))
        .expect("count query runs")
}

#[test]
fn migrates_v3_memory_into_the_v4_store() {
    let mut connection = v3_database();
    assert!(matches!(
        initialize(&mut connection, 200).expect("migration runs"),
        OpenResult::Ready(_)
    ));

    assert_eq!(count(&connection, "PRAGMA user_version"), SCHEMA_VERSION);
    assert_eq!(
        count(
            &connection,
            "SELECT COUNT(*) FROM sqlite_master WHERE name IN (
                'memory_sources', 'source_parts', 'source_dependencies', 'source_suppressions',
                'memory_jobs', 'memory_records', 'memory_evidence', 'memory_state')"
        ),
        0
    );

    // Source ids are preserved; forgotten stays forgotten; the un-backfilled
    // turn gets a new source; the failed turn gets none.
    assert_eq!(
        count(
            &connection,
            "SELECT COUNT(*) FROM sources WHERE id = 1 AND state = 'active'"
        ),
        1
    );
    assert_eq!(
        count(
            &connection,
            "SELECT COUNT(*) FROM sources WHERE id = 2 AND state = 'forgotten'"
        ),
        1
    );
    assert_eq!(
        count(
            &connection,
            "SELECT COUNT(*) FROM sources WHERE kind = 'import' AND display_name = 'garden.md'"
        ),
        1
    );
    assert_eq!(
        count(
            &connection,
            "SELECT COUNT(*) FROM sources WHERE turn_id = 3"
        ),
        1
    );
    assert_eq!(
        count(
            &connection,
            "SELECT COUNT(*) FROM sources WHERE turn_id = 4"
        ),
        0
    );
    assert_eq!(
        count(
            &connection,
            "SELECT COUNT(*) FROM source_links WHERE source_id = 2 AND depends_on_source_id = 1"
        ),
        1
    );

    // One excerpt per active turn, the import chunk, and the active fact; the
    // forgotten turn's fact is dropped and v3 source excerpts are not copied.
    assert_eq!(count(&connection, "SELECT COUNT(*) FROM memories"), 4);
    assert_eq!(
        count(
            &connection,
            "SELECT COUNT(*) FROM memories WHERE kind = 'fact'"
        ),
        1
    );
    assert_eq!(
        count(
            &connection,
            "SELECT COUNT(*) FROM memories WHERE text LIKE '%4321%'"
        ),
        0
    );
    assert_eq!(
        count(
            &connection,
            "SELECT COUNT(*) FROM memories_fts WHERE memories_fts MATCH 'miso'"
        ),
        2
    );
    assert_eq!(
        count(
            &connection,
            "SELECT COUNT(*) FROM memories_fts WHERE memories_fts MATCH 'hiking'"
        ),
        1
    );
    assert_eq!(
        count(
            &connection,
            "SELECT COUNT(*) FROM memories WHERE embedding_model IS NULL"
        ),
        4
    );
}

#[test]
fn fresh_databases_start_at_the_current_version() {
    crate::storage::sqlite_vec::register();
    let mut connection = Connection::open_in_memory().expect("in-memory database opens");
    assert!(matches!(
        initialize(&mut connection, 1).expect("schema initializes"),
        OpenResult::Ready(1)
    ));
    assert_eq!(count(&connection, "PRAGMA user_version"), SCHEMA_VERSION);
    assert_eq!(count(&connection, "SELECT COUNT(*) FROM memories"), 0);
}
