use std::{collections::VecDeque, path::PathBuf};

use rig_core::message::Message;
use tokio::sync::mpsc;

use crate::assistant::{
    Assistant, AssistantError, AssistantRun, AssistantRunError, AssistantTextDelta,
};
use crate::memory::{MemoryService, RecallSnapshot, RecallState};
use crate::storage::{
    ArchiveOutcome, ConversationArchive, MemoryEligibility, MemoryEnqueueState, MemoryRepository,
    SourceId, StorageError,
};

const MAX_EXCHANGES: usize = 3;
const MAX_HISTORY_BYTES: usize = 16 * 1024;

const MISSING_MESSAGES_WARNING: &str =
    "Recent conversation context was cleared because Rig did not return message history.";
const OVERSIZED_BATCH_WARNING: &str = "Recent conversation context was cleared because the latest exchange exceeded the 16 KiB history limit.";
const HISTORY_SERIALIZATION_WARNING: &str =
    "Recent conversation context was cleared because its messages could not be serialized.";
const SUPPRESSED_LINEAGE_WARNING: &str = "Recent conversation context was cleared because a recalled memory changed while the answer was being saved.";

// LEARNING: A Rust struct holds data; its `impl` groups its methods. Java usually keeps data and
// methods together inside a class.
// `pub(crate)` limits access to this Rust crate. Fields without `pub` are private to this module.
// `VecDeque<Vec<Message>>` resembles Java's `ArrayDeque<List<Message>>`: each inner collection owns
// one complete exchange, while the outer queue can discard its oldest exchange.
// Rust's queue owns Message values, not Java-style shared object references.
pub(crate) struct ConversationSession {
    archive_path: PathBuf,
    batches: VecDeque<HistoryBatch>,
    archive: Option<ConversationArchive>,
}

// A whole native Rig message batch stays together with the trusted source
// dependencies that informed it, so forget can evict complete exchanges.
struct HistoryBatch {
    messages: Vec<Message>,
    source_ids: Vec<SourceId>,
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
            batches: VecDeque::new(),
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
        self.batches.clear();
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
        self.batches.clear();
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
        let excluded = self.history_source_ids();
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
        let history = self.history();

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
            self.batches.clear();
            Some(SUPPRESSED_LINEAGE_WARNING)
        } else {
            match &run {
                Ok(run) => self.record_success_with_sources(
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

    fn history(&self) -> Vec<Message> {
        // LEARNING: This resembles Java Streams `flatMap` and collection, but `.cloned().collect()`
        // builds an owned `Vec<Message>`.
        // Here `cloned()` copies each Message value rather than a Java-style shared object reference.
        self.batches
            .iter()
            .flat_map(|batch| batch.messages.iter().cloned())
            .collect()
    }

    fn history_source_ids(&self) -> Vec<SourceId> {
        let mut sources = Vec::new();
        for source_id in self.batches.iter().flat_map(|batch| &batch.source_ids) {
            if !sources.contains(source_id) {
                sources.push(*source_id);
            }
        }
        sources
    }

    fn record_success_with_sources(
        &mut self,
        messages: Option<&[Message]>,
        recalled_source_ids: &[SourceId],
        current_source_id: Option<SourceId>,
    ) -> Option<&'static str> {
        // LEARNING: `Option<&[Message]>` is an optionally present borrowed slice; Rust makes absence
        // explicit instead of using null.
        // `let Some(..) else` handles absence early. The batch is copied only when the session will
        // own it.
        let Some(messages) = messages else {
            self.batches.clear();
            return Some(MISSING_MESSAGES_WARNING);
        };

        // LEARNING: `Result` carries a byte count or serialization error. This match guard checks the
        // cap only when serialization returned a byte count.
        match serialized_batch_bytes(messages) {
            Ok(bytes) if bytes <= MAX_HISTORY_BYTES => {}
            Ok(_) => {
                self.batches.clear();
                return Some(OVERSIZED_BATCH_WARNING);
            }
            Err(_) => {
                self.batches.clear();
                return Some(HISTORY_SERIALIZATION_WARNING);
            }
        }

        let mut source_ids = recalled_source_ids.to_vec();
        if let Some(source_id) = current_source_id
            && !source_ids.contains(&source_id)
        {
            source_ids.push(source_id);
        }
        self.batches.push_back(HistoryBatch {
            messages: messages.to_vec(),
            source_ids,
        });
        while self.batches.len() > MAX_EXCHANGES {
            self.batches.pop_front();
        }

        // LEARNING: `pop_front()` removes the oldest whole exchange. The loop trims batches until the
        // flattened history fits the byte cap.
        loop {
            match serialized_history_bytes(&self.batches) {
                Ok(bytes) if bytes <= MAX_HISTORY_BYTES => return None,
                Ok(_) => {
                    self.batches.pop_front();
                }
                Err(_) => {
                    self.batches.clear();
                    return Some(HISTORY_SERIALIZATION_WARNING);
                }
            }
        }
    }
}

fn merge_source_dependencies(
    history_sources: &[SourceId],
    recalled_sources: &[SourceId],
) -> Vec<SourceId> {
    let mut dependencies = history_sources.to_vec();
    for source_id in recalled_sources {
        if !dependencies.contains(source_id) {
            dependencies.push(*source_id);
        }
    }
    dependencies
}

// LEARNING: This exhaustive match maps detailed run errors to safe archive categories; raw provider
// diagnostics remain in the run and are not archived.
fn archive_outcome(error: &AssistantRunError) -> ArchiveOutcome {
    match &error.error {
        #[cfg(test)]
        AssistantError::Prompt(_) => ArchiveOutcome::PromptFailed,
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

// LEARNING: Borrowed messages are serialized for an exact JSON byte count. The `Result` makes
// serialization failure explicit to the caller.
fn serialized_batch_bytes(messages: &[Message]) -> Result<usize, serde_json::Error> {
    serde_json::to_vec(messages).map(|serialized| serialized.len())
}

// LEARNING: `Vec<&Message>` is a temporary flattened view for counting.
// Sizing history does not clone the messages again.
fn serialized_history_bytes(batches: &VecDeque<HistoryBatch>) -> Result<usize, serde_json::Error> {
    let history: Vec<&Message> = batches
        .iter()
        .flat_map(|batch| batch.messages.iter())
        .collect();
    serde_json::to_vec(&history).map(|serialized| serialized.len())
}

#[cfg(test)]
#[path = "../../tests/unit/conversation/conversation_tests.rs"]
mod tests;
