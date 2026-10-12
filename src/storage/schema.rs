use rusqlite::{Transaction, TransactionBehavior, params};

use super::chunking::{MAX_CHUNK_BYTES, conversation_text, split};

pub(super) const SCHEMA_VERSION: i64 = 4;

const CREATE_SCHEMA_V1: &str = r#"
CREATE TABLE sessions (
    id INTEGER PRIMARY KEY,
    started_at_unix_ms INTEGER NOT NULL
);

CREATE TABLE turns (
    id INTEGER PRIMARY KEY,
    session_id INTEGER NOT NULL REFERENCES sessions(id),
    created_at_unix_ms INTEGER NOT NULL,
    user_text TEXT NOT NULL,
    assistant_text TEXT,
    outcome TEXT NOT NULL
        CHECK (outcome IN (
            'completed',
            'prompt_failed',
            'timed_out',
            'invalid_response'
        )),
    CHECK (
        (outcome = 'completed' AND assistant_text IS NOT NULL)
        OR
        (outcome <> 'completed' AND assistant_text IS NULL)
    )
);

CREATE INDEX turns_by_session ON turns(session_id, id);
PRAGMA user_version = 1;
"#;

const MIGRATE_V1_TO_V2: &str = r#"
ALTER TABLE turns ADD COLUMN memory_eligible INTEGER NOT NULL DEFAULT 1
    CHECK (memory_eligible IN (0, 1));

CREATE TABLE memory_sources (
    id INTEGER PRIMARY KEY,
    kind TEXT NOT NULL CHECK (kind IN ('conversation', 'import', 'action')),
    source_key TEXT NOT NULL,
    revision_sha256 TEXT NOT NULL,
    display_source TEXT,
    turn_id INTEGER REFERENCES turns(id),
    created_at_unix_ms INTEGER NOT NULL,
    state TEXT NOT NULL DEFAULT 'active'
        CHECK (state IN ('active', 'superseded', 'suppressed')),
    extraction_state TEXT NOT NULL DEFAULT 'not_applicable'
        CHECK (extraction_state IN (
            'not_applicable', 'pending', 'queued', 'running', 'failed',
            'complete', 'suppressed'
        )),
    UNIQUE(kind, source_key, revision_sha256),
    CHECK ((kind = 'conversation' AND turn_id IS NOT NULL)
        OR (kind <> 'conversation' AND turn_id IS NULL))
);

CREATE INDEX memory_sources_pending_extraction
    ON memory_sources(extraction_state, id);
CREATE INDEX memory_sources_by_key
    ON memory_sources(kind, source_key, id);

CREATE TABLE source_parts (
    source_id INTEGER NOT NULL REFERENCES memory_sources(id),
    part_index INTEGER NOT NULL CHECK (part_index >= 0),
    role TEXT NOT NULL CHECK (role IN ('user', 'assistant', 'imported', 'action_result')),
    content TEXT,
    projection_state TEXT NOT NULL DEFAULT 'pending'
        CHECK (projection_state IN ('pending', 'complete', 'suppressed')),
    PRIMARY KEY(source_id, part_index),
    CHECK ((role IN ('user', 'assistant') AND content IS NULL)
        OR (role IN ('imported', 'action_result') AND content IS NOT NULL))
);
CREATE INDEX source_parts_pending_projection
    ON source_parts(projection_state, source_id, part_index);

CREATE TABLE source_dependencies (
    source_id INTEGER NOT NULL REFERENCES memory_sources(id),
    depends_on_source_id INTEGER NOT NULL REFERENCES memory_sources(id),
    PRIMARY KEY(source_id, depends_on_source_id),
    CHECK (source_id > depends_on_source_id)
);
CREATE INDEX source_dependencies_by_parent
    ON source_dependencies(depends_on_source_id, source_id);

CREATE TABLE source_suppressions (
    source_id INTEGER PRIMARY KEY REFERENCES memory_sources(id),
    suppressed_at_unix_ms INTEGER NOT NULL,
    reason TEXT NOT NULL CHECK (reason IN ('forgotten_memory', 'forgotten_source'))
);

CREATE TABLE memory_jobs (
    id INTEGER PRIMARY KEY,
    task_key TEXT NOT NULL UNIQUE,
    source_id INTEGER NOT NULL REFERENCES memory_sources(id),
    revision_sha256 TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'queued'
        CHECK (status IN ('queued', 'running', 'failed', 'completed')),
    attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count BETWEEN 0 AND 3),
    lease_token INTEGER NOT NULL DEFAULT 0,
    lease_until_unix_ms INTEGER,
    failure_category TEXT,
    created_at_unix_ms INTEGER NOT NULL,
    updated_at_unix_ms INTEGER NOT NULL
);
CREATE INDEX memory_jobs_queue ON memory_jobs(status, created_at_unix_ms, id);
CREATE UNIQUE INDEX one_running_extraction
    ON memory_jobs(status) WHERE status = 'running';

CREATE TABLE memory_records (
    id INTEGER PRIMARY KEY,
    source_id INTEGER NOT NULL REFERENCES memory_sources(id),
    record_key TEXT UNIQUE,
    text TEXT NOT NULL CHECK (length(CAST(text AS BLOB)) BETWEEN 1 AND 4096),
    record_kind TEXT NOT NULL CHECK (record_kind IN (
        'source_excerpt', 'imported_chunk', 'preference', 'relationship',
        'event', 'project', 'experience', 'other'
    )),
    attribution TEXT NOT NULL CHECK (attribution IN (
        'user_statement', 'assistant_answer', 'imported_claim',
        'action_result', 'inference'
    )),
    status TEXT NOT NULL DEFAULT 'active'
        CHECK (status IN ('active', 'superseded', 'forgotten')),
    created_at_unix_ms INTEGER NOT NULL,
    projection_generation INTEGER NOT NULL DEFAULT 1,
    projected_generation INTEGER NOT NULL DEFAULT 0,
    projection_op TEXT NOT NULL DEFAULT 'upsert'
        CHECK (projection_op IN ('upsert', 'delete')),
    projection_lease_token INTEGER NOT NULL DEFAULT 0,
    projection_lease_until_unix_ms INTEGER,
    projection_failures INTEGER NOT NULL DEFAULT 0,
    projection_retry_after_unix_ms INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX memory_records_by_source ON memory_records(source_id, id);
CREATE INDEX memory_records_projection
    ON memory_records(projected_generation, projection_retry_after_unix_ms, id);

CREATE TABLE memory_evidence (
    record_id INTEGER NOT NULL REFERENCES memory_records(id),
    source_id INTEGER NOT NULL,
    part_index INTEGER NOT NULL,
    start_byte INTEGER NOT NULL CHECK (start_byte >= 0),
    end_byte INTEGER NOT NULL CHECK (end_byte >= start_byte),
    quote TEXT,
    PRIMARY KEY(record_id, source_id, part_index, start_byte, end_byte),
    FOREIGN KEY(source_id, part_index)
        REFERENCES source_parts(source_id, part_index)
);
CREATE INDEX memory_evidence_by_source ON memory_evidence(source_id, part_index);

CREATE TABLE memory_state (
    key TEXT PRIMARY KEY,
    integer_value INTEGER NOT NULL
);
INSERT INTO memory_state(key, integer_value) VALUES ('backfill_cursor_turn_id', 0);
PRAGMA user_version = 2;
"#;

const MIGRATE_V2_TO_V3: &str = r#"
ALTER TABLE memory_records ADD COLUMN correction_state TEXT NOT NULL DEFAULT 'none'
    CHECK (correction_state IN ('none', 'applied', 'needs_review'));
ALTER TABLE memory_records ADD COLUMN supersedes_id INTEGER REFERENCES memory_records(id);
PRAGMA user_version = 3;
"#;

/// v4 replaces the extraction/projection pipeline (jobs, leases, outbox,
/// LanceDB) with one transactional store: `memories` plus an FTS5 keyword index
/// and a sqlite-vec vector table, both kept in sync by triggers.
const CREATE_V4_TABLES: &str = r#"
CREATE TABLE sources (
    id INTEGER PRIMARY KEY,
    kind TEXT NOT NULL CHECK (kind IN ('conversation', 'import')),
    turn_id INTEGER UNIQUE REFERENCES turns(id),
    path_key TEXT,
    display_name TEXT,
    revision_sha256 TEXT,
    state TEXT NOT NULL DEFAULT 'active'
        CHECK (state IN ('active', 'superseded', 'forgotten')),
    created_at_unix_ms INTEGER NOT NULL,
    CHECK ((kind = 'conversation' AND turn_id IS NOT NULL AND path_key IS NULL)
        OR (kind = 'import' AND turn_id IS NULL AND path_key IS NOT NULL
            AND display_name IS NOT NULL AND revision_sha256 IS NOT NULL))
);
CREATE UNIQUE INDEX sources_import_revision
    ON sources(path_key, revision_sha256) WHERE kind = 'import';

-- A conversation source depends on every source recalled into its run, so
-- forgetting a source also forgets answers that were built from it.
CREATE TABLE source_links (
    source_id INTEGER NOT NULL REFERENCES sources(id),
    depends_on_source_id INTEGER NOT NULL REFERENCES sources(id),
    PRIMARY KEY (source_id, depends_on_source_id),
    CHECK (source_id > depends_on_source_id)
);
CREATE INDEX source_links_by_parent ON source_links(depends_on_source_id, source_id);

CREATE TABLE memories (
    id INTEGER PRIMARY KEY,
    source_id INTEGER NOT NULL REFERENCES sources(id),
    kind TEXT NOT NULL CHECK (kind IN ('excerpt', 'fact')),
    text TEXT NOT NULL CHECK (length(CAST(text AS BLOB)) BETWEEN 1 AND 4096),
    created_at_unix_ms INTEGER NOT NULL,
    -- Fingerprint of the model that produced this row's vector; NULL = pending.
    embedding_model TEXT
);
CREATE INDEX memories_by_source ON memories(source_id, id);

CREATE VIRTUAL TABLE memories_fts USING fts5(
    text, content = 'memories', content_rowid = 'id', tokenize = 'porter unicode61'
);
CREATE VIRTUAL TABLE memory_vectors USING vec0(embedding float[384]);

CREATE TRIGGER memories_after_insert AFTER INSERT ON memories BEGIN
    INSERT INTO memories_fts(rowid, text) VALUES (new.id, new.text);
END;
CREATE TRIGGER memories_after_delete AFTER DELETE ON memories BEGIN
    INSERT INTO memories_fts(memories_fts, rowid, text) VALUES ('delete', old.id, old.text);
    DELETE FROM memory_vectors WHERE rowid = old.id;
END;
CREATE TRIGGER memories_text_is_immutable BEFORE UPDATE OF text ON memories BEGIN
    SELECT RAISE(ABORT, 'memory text is immutable');
END;
"#;

const DROP_V3_TABLES: &str = r#"
DROP TABLE memory_evidence;
DROP TABLE memory_records;
DROP TABLE memory_jobs;
DROP TABLE source_suppressions;
DROP TABLE source_dependencies;
DROP TABLE source_parts;
DROP TABLE memory_sources;
DROP TABLE memory_state;
PRAGMA user_version = 4;
"#;

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
