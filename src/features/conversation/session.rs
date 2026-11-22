use std::{path::PathBuf, time::Duration};

use rig_core::message::Message;
use tokio::{sync::mpsc, time::Instant};

use super::history::{History, Recorded, merge_source_dependencies};
use crate::features::assistant::{
    Assistant, AssistantError, AssistantRun, AssistantRunError, AssistantTextDelta, CacheWarmup,
};
use crate::features::memory::{MemoryService, RecallMode, RecallSnapshot, facts_message};
use crate::storage::{
    ArchiveOutcome, ConversationArchive, FactsRepository, MemoryEligibility, MemoryRepository,
    MemorySaveState, SourceId, StorageError, TimersRepository,
};

/// Stands in for the first prompt in the start-up warm-up: the server caches the
/// request up to where this differs from what Darren will say.
const STARTUP_WARMUP_PROMPT: &str = "Hello.";
const SUPPRESSED_LINEAGE_WARNING: &str = "Recent conversation context was cleared because a recalled memory changed while the answer was being saved.";

// LEARNING: A Rust struct holds data; its `impl` groups its methods. Java usually keeps data and
// methods together inside a class.
// `pub(crate)` limits access to this Rust crate. Fields without `pub` stay private to this module.
pub(crate) struct ConversationSession {
    archive_path: PathBuf,
    history: History,
    archive: Option<ConversationArchive>,
    /// When the last successful answer was recorded, on Tokio's monotonic clock so a
    /// wall-clock change cannot move it. `None` until the first one and after a reset.
    last_answer_at: Option<Instant>,
}

// LEARNING: `Result` carries the model outcome; `Option` marks metadata that may be absent,
// similar to Java's `Optional<T>` without storing a nullable value.
// Here, `&'static str` refers to warning string literals that live for the whole program.
// The lifetime describes how long a reference is valid; it does not imply garbage collection.
pub(crate) struct ConversationTurn {
    pub(crate) run: Result<AssistantRun, Box<AssistantRunError>>,
    pub(crate) context_warning: Option<String>,
    pub(crate) archive_error: Option<StorageError>,
    pub(crate) memory_state: Option<MemorySaveState>,
    pub(crate) current_source_id: Option<SourceId>,
    /// Set when this turn trimmed the history: the start of the next request, for
    /// the caller to send ahead with `Assistant::warm_cache`.
    pub(crate) cache_warmup: Option<CacheWarmup>,
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
            last_answer_at: None,
        }
    }

    // LEARNING: `&mut self` is an exclusive mutable borrow. While this call can change the session,
    // Rust prevents another alias from mutating it at the same time; Java references do not
    // enforce this.
    // `Option::take()` moves the old archive out and leaves `None`; its unused return value is dropped
    // before the fresh open.
    // On error the session keeps working with process-only context (no archive).
    pub(crate) async fn reset(&mut self) -> Result<(), StorageError> {
        self.history.clear();
        self.last_answer_at = None;
        let next = match self.archive.take() {
            Some(archive) => archive.next_session().await,
            None => ConversationArchive::open(self.archive_path.clone()).await,
        };
        self.archive = Some(next?);
        Ok(())
    }

    /// How long it has been since the last successful answer was recorded; `None`
    /// before any answer in this conversation. A failed or dropped turn records
    /// nothing, so it never restarts the count.
    pub(crate) fn quiet_for(&self) -> Option<Duration> {
        self.last_answer_at.map(|at| at.elapsed())
    }

    pub(crate) fn memory_repository(&self) -> Option<MemoryRepository> {
        self.archive
            .as_ref()
            .map(ConversationArchive::memory_repository)
    }

    /// Where facts are added and removed: "remember that" and "forget that", and the
    /// idle noting pass (fast-memory 05).
    pub(crate) fn facts_repository(&self) -> Option<FactsRepository> {
        self.archive
            .as_ref()
            .map(ConversationArchive::facts_repository)
    }

    /// Where timers are kept (timers 04); the timer service runs on it.
    pub(crate) fn timers_repository(&self) -> Option<TimersRepository> {
        self.archive
            .as_ref()
            .map(ConversationArchive::timers_repository)
    }

    pub(crate) fn clear_history(&mut self) {
        self.history.clear();
    }

    /// When `recall_mode` is `Search`, retrieves bounded memory once and sends it as
    /// one run-local message before the last whole exchange. `Skip` makes no search
    /// and no warning. Either way the canonical final is archived only after the
    /// native stream completes.
    ///
    /// Every request starts with the facts list, read from storage now, so a fact
    /// added or removed since the last turn shows in this request.
    pub(crate) async fn respond_stream(
        &mut self,
        assistant: &Assistant,
        memory: Option<&MemoryService>,
        recall_mode: RecallMode,
        prompt: &str,
        deltas: mpsc::Sender<AssistantTextDelta>,
    ) -> ConversationTurn {
        let excluded = self.history.source_ids();
        let recall = match (recall_mode, memory) {
            (RecallMode::Skip, _) => RecallSnapshot::default(),
            (RecallMode::Search, Some(memory)) => memory.recall(prompt, &excluded).await,
            (RecallMode::Search, None) => RecallSnapshot::unavailable(
                "local memory storage is unavailable; normal chat is continuing",
            ),
        };
        let (facts, facts_warning) = self.facts_for_request().await;
        let context_warning = join_warnings(recall.warning.clone(), facts_warning);
        let (earlier_history, latest_exchange) = self.history.split_last_exchange();

        // LEARNING: These Documents are request context, not Message history.
        // The assistant sends them as one message between the earlier history and
        // the last exchange, so the cache keeps the older turns and the latest
        // exchange stays next to the question. Rig's returned batch starts at the
        // prompt, so the recall snapshot never enters the replayed conversation
        // while native IDs/correlation are retained.
        let run = assistant
            .respond_stream(
                prompt,
                facts.as_ref(),
                &earlier_history,
                &latest_exchange,
                &recall.documents,
                deltas,
            )
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
        let (current_source, archive_error, lineage_suppressed, memory_state) =
            match self.archive.as_ref() {
                Some(archive) => match archive
                    .append_turn(prompt, outcome, eligibility, &dependencies)
                    .await
                {
                    Ok(receipt) => (
                        receipt.source_id,
                        None,
                        receipt.memory == MemorySaveState::Suppressed,
                        Some(receipt.memory),
                    ),
                    Err(error) => (None, Some(error), false, None),
                },
                None => (None, None, false, None),
            };
        let recorded = if lineage_suppressed {
            // The answer was generated against lineage SQLite rejected as stale;
            // keep it archived for the user, but don't replay unsafe native history.
            self.history.clear();
            Recorded::Cleared(SUPPRESSED_LINEAGE_WARNING)
        } else {
            match &run {
                Ok(run) => self.history.record_success(
                    run.response.messages(),
                    &recall.source_ids,
                    current_source,
                ),
                Err(_) => Recorded::Kept,
            }
        };
        let (history_warning, cache_warmup) = match recorded {
            Recorded::Kept => (None, None),
            Recorded::Trimmed => (None, self.cache_warmup(facts)),
            Recorded::Cleared(warning) => (Some(warning), None),
        };
        let context_warning = combine_warnings(context_warning, history_warning);
        if run.is_ok() {
            self.last_answer_at = Some(Instant::now());
        }
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
            cache_warmup,
        }
    }

    /// The facts message for one request, read from storage, and a warning when
    /// that failed. With no archive there is no list and no warning: startup
    /// already said so, and a turn must not repeat it. A failed read sends the
    /// request without facts; it never fails the turn.
    async fn facts_for_request(&self) -> (Option<Message>, Option<String>) {
        let Some(archive) = self.archive.as_ref() else {
            return (None, None);
        };
        match archive.facts_repository().list().await {
            Ok(facts) => (facts_message(&facts), None),
            Err(error) => (
                None,
                Some(format!(
                    "the facts list is unavailable: {error}; normal chat is continuing"
                )),
            ),
        }
    }

    /// The start of a fresh conversation's first request, to send ahead when Jarvis
    /// starts (fast-memory 13): the facts message and [`STARTUP_WARMUP_PROMPT`] in
    /// place of the prompt nobody has said yet. The server caches the system prompt,
    /// tools and facts before it, which a cold read takes about 30 s to reach. A
    /// failed facts read warms without them; the first turn reports it.
    pub(crate) async fn startup_warmup(&self) -> CacheWarmup {
        let (facts, _warning) = self.facts_for_request().await;
        CacheWarmup::new(Vec::new(), Message::user(STARTUP_WARMUP_PROMPT)).with_facts(facts)
    }

    /// How the next request begins: the facts message, then every stored message up
    /// to and including the last exchange's prompt. The answer after it cannot be
    /// sent ahead, because a request has to end with a prompt. `facts` is what the
    /// turn that just ended sent; the list is read again when the next turn starts.
    fn cache_warmup(&self, facts: Option<Message>) -> Option<CacheWarmup> {
        let (earlier_history, latest_exchange) = self.history.split_last_exchange();
        let prompt = latest_exchange.into_iter().next()?;
        Some(CacheWarmup::new(earlier_history, prompt).with_facts(facts))
    }

    /// Saves a safe, non-eligible outcome when terminal output fails mid-stream.
    pub(crate) async fn record_interrupted(&self, prompt: &str) -> Result<(), StorageError> {
        if let Some(archive) = self.archive.as_ref() {
            archive
                .append_turn(
                    prompt,
                    ArchiveOutcome::PromptFailed,
                    MemoryEligibility::ArchiveOnly,
                    &[],
                )
                .await?;
        }
        Ok(())
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

fn join_warnings(first: Option<String>, second: Option<String>) -> Option<String> {
    match (first, second) {
        (Some(first), Some(second)) => Some(format!("{first} {second}")),
        (Some(only), None) | (None, Some(only)) => Some(only),
        (None, None) => None,
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
#[path = "../../../tests/unit/features/conversation/session_tests.rs"]
mod tests;
