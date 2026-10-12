use std::path::PathBuf;

use tokio::sync::mpsc;

use super::history::{History, merge_source_dependencies};
use crate::features::assistant::{
    Assistant, AssistantError, AssistantRun, AssistantRunError, AssistantTextDelta,
};
use crate::features::memory::{MemoryService, RecallSnapshot, RecallState};
use crate::storage::{
    ArchiveOutcome, ConversationArchive, MemoryEligibility, MemoryEnqueueState, MemoryRepository,
    SourceId, StorageError,
};

const SUPPRESSED_LINEAGE_WARNING: &str = "Recent conversation context was cleared because a recalled memory changed while the answer was being saved.";

// LEARNING: A Rust struct holds data; its `impl` groups its methods. Java usually keeps data and
// methods together inside a class.
// `pub(crate)` limits access to this Rust crate. Fields without `pub` stay private to this module.
pub(crate) struct ConversationSession {
    archive_path: PathBuf,
    history: History,
    archive: Option<ConversationArchive>,
}

// LEARNING: `Result` carries the model outcome; `Option` marks metadata that may be absent,
// similar to Java's `Optional<T>` without storing a nullable value.
// Here, `&'static str` refers to warning string literals that live for the whole program.
// The lifetime describes how long a reference is valid; it does not imply garbage collection.
pub(crate) struct ConversationTurn {
    pub(crate) run: Result<AssistantRun, Box<AssistantRunError>>,
    pub(crate) context_warning: Option<String>,
    pub(crate) archive_error: Option<StorageError>,
    pub(crate) memory_state: Option<MemoryEnqueueState>,
    pub(crate) current_source_id: Option<SourceId>,
}

impl ConversationSession {
    // LEARNING: `impl Into<PathBuf>` accepts anything convertible into an owned path, including
    // borrowed text or path inputs, and stores the converted value in the session.
    // `PathBuf` owns the filesystem path; `&str` elsewhere is a borrowed view of text.
    pub(crate) fn new(archive_path: impl Into<PathBuf>) -> Self {
        Self {
            archive_path: archive_path.into(),
            history: History::default(),
            archive: None,
        }
    }

    // LEARNING: `&mut self` is an exclusive mutable borrow. While this call can change the session,
    // Rust prevents another alias from mutating it at the same time; Java references do not
    // enforce this.
    // `Option::take()` moves the old archive out and leaves `None`; its unused return value is dropped
    // before the fresh open.
    // `Option<StorageError>` means reset returns either a typed error or no error, not a nullable error.
    pub(crate) async fn reset(&mut self) -> Option<StorageError> {
        self.history.clear();
        let next = match self.archive.take() {
            Some(archive) => archive.next_session().await,
            None => ConversationArchive::open(self.archive_path.clone()).await,
        };

        match next {
            Ok(archive) => {
                self.archive = Some(archive);
                None
            }
            Err(error) => Some(error),
        }
    }

    pub(crate) fn memory_repository(&self) -> Option<MemoryRepository> {
        self.archive
            .as_ref()
            .map(ConversationArchive::memory_repository)
    }

    pub(crate) fn clear_history(&mut self) {
        self.history.clear();
    }

    /// Retrieves bounded memory once, sends it as run-local Rig documents, and
    /// archives the canonical final only after the native stream completes.
    pub(crate) async fn respond_stream(
        &mut self,
        assistant: &Assistant,
        memory: Option<&mut MemoryService>,
        prompt: &str,
        deltas: mpsc::Sender<AssistantTextDelta>,
    ) -> ConversationTurn {
        let excluded = self.history.source_ids();
        let recall = match memory {
            Some(memory) => memory.recall(prompt, &excluded).await,
            None => RecallSnapshot::unavailable(
                "local memory storage is unavailable; normal chat is continuing",
            ),
        };
        let context_warning = match &recall.state {
            RecallState::Available | RecallState::Empty => None,
            RecallState::Unavailable(message) => Some(message.clone()),
        };
        let history = self.history.messages();

        // LEARNING: These Documents are request context, not Message history.
        // The native transcript retains Rig IDs/correlation while this bounded
        // recall snapshot stays out of the replayed conversation batch.
        let run = assistant
            .respond_stream(prompt, &history, &recall.documents, deltas)
            .await;

        let (outcome, eligibility) = match &run {
            Ok(run) => (
                ArchiveOutcome::Completed(run.response.output().to_owned()),
                MemoryEligibility::Eligible,
            ),
            Err(error) => (archive_outcome(error), MemoryEligibility::ArchiveOnly),
        };
        let dependencies = if run.is_ok() {
            merge_source_dependencies(&excluded, &recall.source_ids)
        } else {
            Vec::new()
        };
        let (current_source, archive_error, lineage_suppressed, memory_state) = match self
            .archive
            .as_ref()
        {
            Some(archive) => match archive
                .append_turn_with_memory_and_sources(prompt, outcome, eligibility, &dependencies)
                .await
            {
                Ok(receipt) => (
                    receipt.source_id,
                    None,
                    receipt.memory == MemoryEnqueueState::Suppressed,
                    Some(receipt.memory),
                ),
                Err(error) => (None, Some(error), false, None),
            },
            None => (None, None, false, None),
        };
        let history_warning = if lineage_suppressed {
            // The answer was generated against lineage SQLite rejected as stale;
            // keep it archived for the user, but don't replay unsafe native history.
            self.history.clear();
            Some(SUPPRESSED_LINEAGE_WARNING)
        } else {
            match &run {
                Ok(run) => self.history.record_success_with_sources(
                    run.response.messages(),
                    &recall.source_ids,
                    current_source,
                ),
                Err(_) => None,
            }
        };
        let context_warning = combine_warnings(context_warning, history_warning);
        let current_source_id = if run.is_ok() && !lineage_suppressed {
            current_source
        } else {
            None
        };

        ConversationTurn {
            run,
            context_warning,
            archive_error,
            memory_state,
            current_source_id,
        }
    }

    /// Saves a safe, non-eligible outcome when terminal output fails mid-stream.
    pub(crate) async fn record_interrupted(&self, prompt: &str) -> Option<StorageError> {
        match self.archive.as_ref() {
            Some(archive) => archive
                .append_turn_with_memory_and_sources(
                    prompt,
                    ArchiveOutcome::PromptFailed,
                    MemoryEligibility::ArchiveOnly,
                    &[],
                )
                .await
                .err(),
            None => None,
        }
    }
}

// LEARNING: This exhaustive match maps detailed run errors to safe archive categories; raw provider
// diagnostics remain in the run and are not archived.
fn archive_outcome(error: &AssistantRunError) -> ArchiveOutcome {
    match &error.error {
        AssistantError::Stream(_)
        | AssistantError::MissingFinalResponse
        | AssistantError::DeltaReceiverClosed => ArchiveOutcome::PromptFailed,
        AssistantError::RunTimeout => ArchiveOutcome::TimedOut,
        AssistantError::InvalidResponse => ArchiveOutcome::InvalidResponse,
    }
}

fn combine_warnings(
    memory_warning: Option<String>,
    history_warning: Option<&'static str>,
) -> Option<String> {
    match (memory_warning, history_warning) {
        (Some(memory), Some(history)) => Some(format!("{memory} {history}")),
        (Some(memory), None) => Some(memory),
        (None, Some(history)) => Some(history.to_owned()),
        (None, None) => None,
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/conversation/conversation_tests.rs"]
mod tests;
