CREATE TABLE facts (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    text TEXT NOT NULL CHECK (length(CAST(text AS BLOB)) >= 1),
    origin TEXT NOT NULL CHECK (origin IN ('stated', 'noted')),
    created_at_unix_ms INTEGER NOT NULL
);
PRAGMA user_version = 5;
