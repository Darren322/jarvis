use tokio_rusqlite::Connection;

mod memories;
mod sources;

pub(super) use sources::{MAX_RECALLED_SOURCES, insert_conversation_source};

/// SQL for long-term memory: sources (turns and imports), their chunked
/// memories, and the FTS5/sqlite-vec indexes kept in sync by schema triggers.
#[derive(Clone)]
pub(crate) struct MemoryRepository {
    pub(super) connection: Connection,
}

#[cfg(test)]
#[path = "../../../tests/unit/storage/memory_repository/memory_repository_tests.rs"]
mod tests;
