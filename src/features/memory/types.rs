use rig_core::completion::Document;

use crate::storage::{MemoryStats, SourceId};

use super::embedder::{EmbedError, EmbedderState};

#[derive(Clone, Debug, Default)]
pub(crate) struct RecallSnapshot {
    pub(crate) documents: Vec<Document>,
    /// Source IDs are aligned with `documents` and become the turn's lineage.
    pub(crate) source_ids: Vec<SourceId>,
    /// Set when recall was degraded (for example keyword-only) or failed.
    pub(crate) warning: Option<String>,
}

impl RecallSnapshot {
    pub(crate) fn unavailable(message: impl Into<String>) -> Self {
        Self {
            warning: Some(message.into()),
            ..Self::default()
        }
    }
}

/// Outcome of one bounded idle pass over pending embeddings.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct IdlePass {
    pub(crate) embedded: usize,
}

impl IdlePass {
    pub(crate) fn has_made_progress(self) -> bool {
        self.embedded > 0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MemoryStatus {
    pub(crate) embedder: EmbedderState,
    pub(crate) stats: MemoryStats,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum MemoryError {
    #[error(transparent)]
    Storage(#[from] crate::storage::StorageError),
    #[error(transparent)]
    Embedding(#[from] EmbedError),
    #[error("memory operation I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("import must be a regular UTF-8 .txt or .md file no larger than 1 MiB")]
    InvalidImport,
    #[error("memory source is too large")]
    SourceTooLarge,
}
