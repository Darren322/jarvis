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
