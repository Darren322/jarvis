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
