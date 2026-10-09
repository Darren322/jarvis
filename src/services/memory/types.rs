use rig_core::completion::{Document, Usage};

use crate::storage::{JobId, MemoryStats, SourceId};

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RecallState {
    Available,
    Empty,
    Unavailable(String),
}

#[derive(Clone, Debug)]
pub(crate) struct RecallSnapshot {
    pub(crate) documents: Vec<Document>,
    /// Source IDs are aligned with `documents` and prevent re-indexing recalled facts.
    pub(crate) source_ids: Vec<SourceId>,
    pub(crate) state: RecallState,
}

impl RecallSnapshot {
    pub(crate) fn empty() -> Self {
        Self {
            documents: Vec::new(),
            source_ids: Vec::new(),
            state: RecallState::Empty,
        }
    }

    pub(crate) fn unavailable(message: impl Into<String>) -> Self {
        Self {
            documents: Vec::new(),
            source_ids: Vec::new(),
            state: RecallState::Unavailable(message.into()),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct JobRun {
    pub(crate) job_id: Option<JobId>,
    pub(crate) attempt: Option<u8>,
    pub(crate) committed_records: usize,
    pub(crate) source_parts_processed: usize,
    pub(crate) backfill_turns_examined: usize,
    pub(crate) projection_updates: usize,
    pub(crate) model_calls: u8,
    /// Unavailable for rejected, failed, timed-out, or unmetered extraction runs.
    pub(crate) usage: Option<Usage>,
}

impl JobRun {
    /// Tells App whether another bounded idle unit can make progress immediately.
    pub(crate) fn made_progress(self) -> bool {
        self.job_id.is_some()
            || self.committed_records > 0
            || self.source_parts_processed > 0
            || self.backfill_turns_examined > 0
            || self.projection_updates > 0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ImportReceipt {
    pub(crate) source_id: SourceId,
    pub(crate) revision_sha256: String,
    pub(crate) state: String,
    pub(crate) parts: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MemoryStatus {
    pub(crate) backend_available: bool,
    pub(crate) warning: Option<String>,
    pub(crate) stats: MemoryStats,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum MemoryError {
    #[error(transparent)]
    Storage(#[from] crate::storage::StorageError),
    #[error("local memory index is unavailable: {0}")]
    Backend(String),
    #[error("a previous local native worker is still running")]
    WorkerBusy,
    #[error("memory worker failed: {0}")]
    Worker(String),
    #[error("memory operation I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("import must be a regular UTF-8 .txt or .md file no larger than 1 MiB")]
    InvalidImport,
    #[error("memory source is too large")]
    SourceTooLarge,
}
