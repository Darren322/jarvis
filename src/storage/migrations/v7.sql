CREATE TABLE timers (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    label TEXT,
    ends_at_unix_ms INTEGER NOT NULL,
    created_at_unix_ms INTEGER NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('pending', 'running', 'ended', 'cancelled'))
);
CREATE INDEX timers_by_state_and_end ON timers(state, ends_at_unix_ms);
PRAGMA user_version = 7;
