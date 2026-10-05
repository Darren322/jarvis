use std::{
    path::Path,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use rusqlite::{TransactionBehavior, params};
use tokio_rusqlite::Connection;

const SCHEMA_VERSION: i64 = 1;
// LEARNING: `r#"..."#` is a raw string literal, so SQL quotes and backslashes
// stay ordinary characters. Like a Java text block, it keeps multiline SQL
// readable, though Java text blocks still process escapes.
const CREATE_SCHEMA: &str = r#"
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

// LEARNING: `thiserror::Error` is a derive macro: its attributes generate the
// formatting and standard error-source plumbing we would otherwise implement.
// Java comparison: this resembles writing an exception's message and cause
// methods, but Rust returns errors as values rather than throwing exceptions.
// `#[from]` generates a conversion from each source error, so `?` can return it
// early as `StorageError` while retaining the original error as its source.
#[derive(Debug, thiserror::Error)]
pub(crate) enum StorageError {
    #[error("archive I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("SQLite worker error: {0}")]
    Worker(#[from] tokio_rusqlite::Error<rusqlite::Error>),
    #[error("unsupported archive schema version: {0}")]
    UnsupportedSchemaVersion(i64),
}

// LEARNING: `pub(crate)` makes the type available inside this crate, while its
// fields remain private to this module tree. The `impl` block below attaches
// methods to this struct; it is not a separate manager object.
pub(crate) struct ConversationArchive {
    connection: Connection,
    session_id: i64,
}

impl ConversationArchive {
    // LEARNING: `impl AsRef<Path>` accepts `&Path`, `PathBuf`, and `&str`
    // through a borrowed `Path` view. This is a compile-time trait
    // bound, roughly like a Java generic method bounded by an interface.
    // `Result<T, E>` makes success (`Ok`) or failure (`Err`) explicit; `?`
    // returns an error early, converting it to `StorageError` when needed.
    pub(crate) async fn open(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        let path = path.as_ref();
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            tokio::fs::create_dir_all(parent).await?;
        }

        let started_at_unix_ms = unix_time_ms()?;
        // LEARNING: In tokio-rusqlite 0.8, SQLite work runs on a background
        // thread. `.await` can suspend this Tokio task; unlike Java
        // `Future.get()`, it does not synchronously block the calling thread.
        // The library schedules the work; this module supplies its schema and
        // chooses which conversation fields to persist.
        let connection = Connection::open(path).await?;
        let initialized = connection
            .call(move |connection| {
                // LEARNING: This timeout is SQLite's wait for a database lock
                // on its worker thread, separate from awaiting the result.
                connection.busy_timeout(Duration::from_secs(1))?;
                connection.execute_batch("PRAGMA foreign_keys = ON;")?;

                // LEARNING: One immediate transaction covers the version
                // check, possible schema creation, and session insert; it asks
                // SQLite for write access at transaction start. An explicit
                // `commit()` keeps them atomic. If dropped first, rusqlite
                // rolls back, like Java cleanup that rolls back an uncommitted
                // transaction.
                let transaction =
                    connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                // LEARNING: `[]` means this SQL has no bound parameters. The
                // `i64` on `version` lets Rust infer `row.get(0)`'s return type.
                let version: i64 =
                    transaction.query_row("PRAGMA user_version", [], |row| row.get(0))?;

                match version {
                    0 => transaction.execute_batch(CREATE_SCHEMA)?,
                    SCHEMA_VERSION => {}
                    version => return Ok(OpenResult::Unsupported(version)),
                }

                transaction.execute(
                    "INSERT INTO sessions (started_at_unix_ms) VALUES (?1)",
                    [started_at_unix_ms],
                )?;

                let session_id = transaction.last_insert_rowid();
                transaction.commit()?;

                Ok(OpenResult::Ready(session_id))
            })
            .await?;

        let session_id = match initialized {
            OpenResult::Ready(session_id) => session_id,
            OpenResult::Unsupported(version) => {
                return Err(StorageError::UnsupportedSchemaVersion(version));
            }
        };

        Ok(Self {
            connection,
            session_id,
        })
    }

    pub(crate) async fn append_turn(
        &self,
        user_text: &str,
        outcome: ArchiveOutcome,
    ) -> Result<(), StorageError> {
        let created_at_unix_ms = unix_time_ms()?;
        let session_id = self.session_id;
        // LEARNING: `call` requires a `Send + 'static` worker closure, so it
        // cannot keep this method's borrowed `&str` alive after return.
        // `to_owned()` copies its text into an independent `String` that the
        // `move` closure below can transfer to the worker; no borrowed pointer
        // is being made unsafe or extended past its lifetime.
        let user_text = user_text.to_owned();
        // LEARNING: `match` handles every outcome explicitly. `Option<String>`
        // represents the optional answer: `Some(text)` is stored as text, and
        // `None` becomes SQL `NULL`; the schema checks this pairing as well.
        let (outcome, assistant_text) = match outcome {
            ArchiveOutcome::Completed(text) => ("completed", Some(text)),
            ArchiveOutcome::PromptFailed => ("prompt_failed", None),
            ArchiveOutcome::TimedOut => ("timed_out", None),
            ArchiveOutcome::InvalidResponse => ("invalid_response", None),
        };

        self.connection
            .call(move |connection| {
                connection.execute(
                    r#"
                    INSERT INTO turns (
                        session_id,
                        created_at_unix_ms,
                        user_text,
                        assistant_text,
                        outcome
                    )
                    VALUES (?1, ?2, ?3, ?4, ?5)
                    "#,
                    // LEARNING: `params!` binds values to `?N` placeholders
                    // in the SQL statement; it does not interpolate SQL text.
                    params![
                        session_id,
                        created_at_unix_ms,
                        user_text,
                        assistant_text,
                        outcome,
                    ],
                )?;

                Ok(())
            })
            .await?;

        Ok(())
    }
}

// LEARNING: Rust enum variants can each carry their own data: `Completed`
// owns its answer text. The Java analogue is a sealed hierarchy of variant
// records, not a plain Java `enum` list of constants.
pub(crate) enum ArchiveOutcome {
    Completed(String),
    PromptFailed,
    TimedOut,
    InvalidResponse,
}

enum OpenResult {
    Ready(i64),
    Unsupported(i64),
}

fn unix_time_ms() -> Result<i64, StorageError> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(std::io::Error::other)?;

    Ok(duration.as_millis() as i64)
}

#[cfg(test)]
#[path = "../tests/unit/storage_tests.rs"]
mod tests;
