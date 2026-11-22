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
