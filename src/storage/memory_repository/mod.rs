use tokio_rusqlite::Connection;

mod corrections;
mod jobs;
mod records;
mod sources;
mod suppression;

pub(super) use sources::{MAX_RECALLED_SOURCES, insert_conversation_source};

#[derive(Clone)]
pub(crate) struct MemoryRepository {
    pub(super) connection: Connection,
}
