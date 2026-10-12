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
    #[error("invalid memory source input: {0}")]
    InvalidInput(&'static str),
    #[error("memory source exceeds the {0}-byte limit")]
    SourceTooLarge(usize),
}
