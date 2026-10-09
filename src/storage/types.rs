#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct SourceId(pub(crate) i64);

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct MemoryId(pub(crate) i64);

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct JobId(pub(crate) i64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MemoryEligibility {
    Eligible,
    ArchiveOnly,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SourceKind {
    Conversation,
    Import,
    Action,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SourcePartRole {
    User,
    Assistant,
    Imported,
    ActionResult,
}

impl SourcePartRole {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::Imported => "imported",
            Self::ActionResult => "action_result",
        }
    }

    pub(super) fn from_str(value: &str) -> Option<Self> {
        match value {
            "user" => Some(Self::User),
            "assistant" => Some(Self::Assistant),
            "imported" => Some(Self::Imported),
            "action_result" => Some(Self::ActionResult),
            _ => None,
        }
    }

    pub(super) fn attribution(self) -> MemoryAttribution {
        match self {
            Self::User => MemoryAttribution::UserStatement,
            Self::Assistant => MemoryAttribution::AssistantAnswer,
            Self::Imported => MemoryAttribution::ImportedClaim,
            Self::ActionResult => MemoryAttribution::ActionResult,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MemoryRecordKind {
    SourceExcerpt,
    ImportedChunk,
    Preference,
    Relationship,
    Event,
    Project,
    Experience,
    Other,
}

impl MemoryRecordKind {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::SourceExcerpt => "source_excerpt",
            Self::ImportedChunk => "imported_chunk",
            Self::Preference => "preference",
            Self::Relationship => "relationship",
            Self::Event => "event",
            Self::Project => "project",
            Self::Experience => "experience",
            Self::Other => "other",
        }
    }

    pub(super) fn from_str(value: &str) -> Option<Self> {
        match value {
            "source_excerpt" => Some(Self::SourceExcerpt),
            "imported_chunk" => Some(Self::ImportedChunk),
            "preference" => Some(Self::Preference),
            "relationship" => Some(Self::Relationship),
            "event" => Some(Self::Event),
            "project" => Some(Self::Project),
            "experience" => Some(Self::Experience),
            "other" => Some(Self::Other),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MemoryAttribution {
    UserStatement,
    AssistantAnswer,
    ImportedClaim,
    ActionResult,
    Inference,
}

impl MemoryAttribution {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::UserStatement => "user_statement",
            Self::AssistantAnswer => "assistant_answer",
            Self::ImportedClaim => "imported_claim",
            Self::ActionResult => "action_result",
            Self::Inference => "inference",
        }
    }

    pub(super) fn from_str(value: &str) -> Option<Self> {
        match value {
            "user_statement" => Some(Self::UserStatement),
            "assistant_answer" => Some(Self::AssistantAnswer),
            "imported_claim" => Some(Self::ImportedClaim),
            "action_result" => Some(Self::ActionResult),
            "inference" => Some(Self::Inference),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MemoryRecordStatus {
    Active,
    Superseded,
    Forgotten,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CorrectionMarker {
    Actually,
    Instead,
    InsteadOf,
    RatherThan,
    IMeant,
    Correction,
    NoLonger,
}

impl CorrectionMarker {
    pub(super) fn phrase(self) -> &'static str {
        match self {
            Self::Actually => "actually",
            Self::Instead => "instead",
            Self::InsteadOf => "instead of",
            Self::RatherThan => "rather than",
            Self::IMeant => "i meant",
            Self::Correction => "correction",
            Self::NoLonger => "no longer",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CorrectionState {
    None,
    Applied,
    NeedsReview,
}

impl CorrectionState {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Applied => "applied",
            Self::NeedsReview => "needs_review",
        }
    }

    pub(super) fn from_str(value: &str) -> Option<Self> {
        match value {
            "none" => Some(Self::None),
            "applied" => Some(Self::Applied),
            "needs_review" => Some(Self::NeedsReview),
            _ => None,
        }
    }
}

impl MemoryRecordStatus {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Superseded => "superseded",
            Self::Forgotten => "forgotten",
        }
    }

    pub(super) fn from_str(value: &str) -> Option<Self> {
        match value {
            "active" => Some(Self::Active),
            "superseded" => Some(Self::Superseded),
            "forgotten" => Some(Self::Forgotten),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MemoryEnqueueState {
    NotEligible,
    Queued,
    Pending,
    Suppressed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ArchiveReceipt {
    pub(crate) turn_id: i64,
    pub(crate) source_id: Option<SourceId>,
    pub(crate) memory: MemoryEnqueueState,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SourcePart {
    pub(crate) index: u32,
    pub(crate) role: SourcePartRole,
    pub(crate) text: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SourceMaterial {
    pub(crate) id: SourceId,
    pub(crate) kind: SourceKind,
    pub(crate) source_key: String,
    pub(crate) revision_sha256: String,
    pub(crate) created_at_unix_ms: i64,
    pub(crate) display_source: Option<String>,
    pub(crate) parts: Vec<SourcePart>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum SourceReceiptState {
    Created,
    AlreadyCurrent,
    Reactivated,
    SuppressedRevision,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SourceReceipt {
    pub(crate) source_id: SourceId,
    pub(crate) revision_sha256: String,
    pub(crate) state: SourceReceiptState,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SourceChunk {
    pub(crate) chunk_index: u32,
    pub(crate) start_byte: u32,
    pub(crate) end_byte: u32,
    pub(crate) text: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PendingSourcePart {
    pub(crate) source: SourceMaterial,
    pub(crate) part: SourcePart,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct EvidenceInput {
    pub(crate) part_index: u32,
    pub(crate) start_byte: u32,
    pub(crate) end_byte: u32,
    pub(crate) quote: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CorrectionEvidence {
    pub(crate) marker: CorrectionMarker,
    pub(crate) subject: EvidenceInput,
    pub(crate) property: EvidenceInput,
    pub(crate) old_value: EvidenceInput,
    pub(crate) new_value: EvidenceInput,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NewMemory {
    pub(crate) text: String,
    pub(crate) kind: MemoryRecordKind,
    pub(crate) attribution: MemoryAttribution,
    pub(crate) evidence: Vec<EvidenceInput>,
    pub(crate) correction: Option<CorrectionEvidence>,
    pub(crate) correction_needs_review: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MemoryEvidence {
    pub(crate) source_id: SourceId,
    pub(crate) part_index: u32,
    pub(crate) source_kind: SourceKind,
    pub(crate) source_name: Option<String>,
    pub(crate) role: SourcePartRole,
    pub(crate) start_byte: u32,
    pub(crate) end_byte: u32,
    pub(crate) quote: Option<String>,
    pub(crate) source_created_at_unix_ms: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct HydratedMemory {
    pub(crate) id: MemoryId,
    pub(crate) text: String,
    pub(crate) kind: MemoryRecordKind,
    pub(crate) attribution: MemoryAttribution,
    pub(crate) status: MemoryRecordStatus,
    pub(crate) correction_state: CorrectionState,
    pub(crate) supersedes_id: Option<MemoryId>,
    pub(crate) source_created_at_unix_ms: i64,
    pub(crate) evidence: Vec<MemoryEvidence>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MemoryJobLease {
    pub(crate) id: JobId,
    pub(crate) source_id: SourceId,
    pub(crate) revision_sha256: String,
    pub(crate) lease_token: i64,
    pub(crate) attempt: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CommitDisposition {
    Committed,
    Empty,
    Suppressed,
    SourceChanged,
    LeaseLost,
    InvalidEvidence,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SafeFailure {
    ExtractionUnavailable,
    InvalidExtraction,
    SourceTooLarge,
}

impl SafeFailure {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::ExtractionUnavailable => "extraction_unavailable",
            Self::InvalidExtraction => "invalid_extraction",
            Self::SourceTooLarge => "source_too_large",
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ForgetReceipt {
    pub(crate) forgotten_memories: Vec<MemoryId>,
    pub(crate) suppressed_sources: Vec<SourceId>,
    pub(crate) affected_turn_ids: Vec<i64>,
    pub(crate) forgotten_memory_count: usize,
    pub(crate) suppressed_source_count: usize,
    pub(crate) affected_turn_count: usize,
    pub(crate) ids_truncated: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RetryDisposition {
    Requeued,
    Pending,
    AttemptsExhausted,
    NotRetryable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProjectionOperation {
    Upsert,
    Delete,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProjectionLease {
    pub(crate) id: MemoryId,
    pub(crate) source_id: SourceId,
    pub(crate) source_revision_sha256: String,
    pub(crate) source_kind: SourceKind,
    pub(crate) source_name: Option<String>,
    pub(crate) created_at_unix_ms: i64,
    pub(crate) generation: i64,
    pub(crate) lease_token: i64,
    pub(crate) operation: ProjectionOperation,
    pub(crate) text: String,
    pub(crate) kind: MemoryRecordKind,
    pub(crate) attribution: MemoryAttribution,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProjectionOutcome {
    Applied,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProjectionPage {
    pub(crate) items: Vec<ProjectionLease>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct MemoryListFilter {
    pub(crate) query: Option<String>,
    pub(crate) status: Option<MemoryRecordStatus>,
    pub(crate) limit: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MemorySummary {
    pub(crate) id: MemoryId,
    pub(crate) source_id: SourceId,
    pub(crate) text: String,
    pub(crate) kind: MemoryRecordKind,
    pub(crate) attribution: MemoryAttribution,
    pub(crate) status: MemoryRecordStatus,
    pub(crate) correction_state: CorrectionState,
    pub(crate) supersedes_id: Option<MemoryId>,
    pub(crate) source_kind: SourceKind,
    pub(crate) source_name: Option<String>,
    pub(crate) created_at_unix_ms: i64,
    pub(crate) source_created_at_unix_ms: i64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct MemoryStats {
    pub(crate) queued_extractions: usize,
    pub(crate) running_extractions: usize,
    pub(crate) failed_extractions: usize,
    pub(crate) pending_sources: usize,
    pub(crate) pending_source_parts: usize,
    pub(crate) pending_projections: usize,
    pub(crate) backfill_complete: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BackfillPage {
    pub(crate) examined_turns: usize,
    pub(crate) created_sources: usize,
    pub(crate) cursor_turn_id: i64,
    pub(crate) complete: bool,
}
