//! Typed values for memory sources, memories, and their receipts.

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct SourceId(pub(crate) i64);

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct MemoryId(pub(crate) i64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SourceKind {
    Conversation,
    Import,
}

impl SourceKind {
    pub(super) fn from_sql(value: &str) -> Result<Self, rusqlite::Error> {
        match value {
            "conversation" => Ok(Self::Conversation),
            "import" => Ok(Self::Import),
            _ => Err(rusqlite::Error::InvalidQuery),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MemoryKind {
    /// Verbatim text from a conversation turn or an imported file.
    Excerpt,
    /// A fact extracted by the retired LLM extraction pipeline (schema v3).
    Fact,
}

impl MemoryKind {
    pub(super) fn from_sql(value: &str) -> Result<Self, rusqlite::Error> {
        match value {
            "excerpt" => Ok(Self::Excerpt),
            "fact" => Ok(Self::Fact),
            _ => Err(rusqlite::Error::InvalidQuery),
        }
    }
}

/// What happened to the memory side of an archived turn.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MemorySaveState {
    /// Failed or archive-only turns are not remembered.
    NotEligible,
    /// The turn is searchable by keyword now and by meaning once embedded.
    Saved,
    /// A memory this answer depended on was forgotten while it was generated.
    Suppressed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ImportState {
    Created,
    AlreadyCurrent,
    Reactivated,
    /// This exact revision was forgotten earlier and stays forgotten.
    Forgotten,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ImportReceipt {
    pub(crate) source_id: SourceId,
    pub(crate) state: ImportState,
    pub(crate) chunks: usize,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ForgetReceipt {
    pub(crate) forgotten_memories: usize,
    pub(crate) forgotten_sources: usize,
    pub(crate) affected_turns: usize,
}

impl ForgetReceipt {
    pub(crate) fn is_empty(&self) -> bool {
        self.forgotten_memories == 0 && self.forgotten_sources == 0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MemorySummary {
    pub(crate) id: MemoryId,
    pub(crate) source_id: SourceId,
    pub(crate) kind: MemoryKind,
    pub(crate) text: String,
}

/// A memory hydrated for recall, with enough provenance to label it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RecalledMemory {
    pub(crate) id: MemoryId,
    pub(crate) source_id: SourceId,
    pub(crate) source_kind: SourceKind,
    pub(crate) source_name: Option<String>,
    pub(crate) source_created_at_unix_ms: i64,
    pub(crate) kind: MemoryKind,
    pub(crate) text: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct MemoryStats {
    pub(crate) memories: usize,
    pub(crate) pending_embeddings: usize,
    pub(crate) conversation_sources: usize,
    pub(crate) import_sources: usize,
}
