use std::{collections::VecDeque, path::PathBuf};

use rig_core::message::Message;

use crate::services::assistant::{Assistant, AssistantError, AssistantRun, AssistantRunError};
use crate::storage::{ArchiveOutcome, ConversationArchive, StorageError};

const MAX_EXCHANGES: usize = 3;
const MAX_HISTORY_BYTES: usize = 16 * 1024;

const MISSING_MESSAGES_WARNING: &str =
    "Recent conversation context was cleared because Rig did not return message history.";
const OVERSIZED_BATCH_WARNING: &str = "Recent conversation context was cleared because the latest exchange exceeded the 16 KiB history limit.";
const HISTORY_SERIALIZATION_WARNING: &str =
    "Recent conversation context was cleared because its messages could not be serialized.";

// LEARNING: A Rust struct holds data; its `impl` groups its methods. Java usually keeps data and
// methods together inside a class.
// `pub(crate)` limits access to this Rust crate. Fields without `pub` are private to this module.
// `VecDeque<Vec<Message>>` resembles Java's `ArrayDeque<List<Message>>`: each inner collection owns
// one complete exchange, while the outer queue can discard its oldest exchange.
// Rust's queue owns Message values, not Java-style shared object references.
pub(crate) struct ConversationSession {
    archive_path: PathBuf,
    batches: VecDeque<Vec<Message>>,
    archive: Option<ConversationArchive>,
}

// LEARNING: `Result` carries the model outcome; `Option` marks metadata that may be absent,
// similar to Java's `Optional<T>` without storing a nullable value.
// Here, `&'static str` refers to warning string literals that live for the whole program.
// The lifetime describes how long a reference is valid; it does not imply garbage collection.
pub(crate) struct ConversationTurn {
    pub(crate) run: Result<AssistantRun, Box<AssistantRunError>>,
    pub(crate) context_warning: Option<&'static str>,
    pub(crate) archive_error: Option<StorageError>,
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
        self.archive.take();

        match ConversationArchive::open(self.archive_path.clone()).await {
            Ok(archive) => {
                self.archive = Some(archive);
                None
            }
            Err(error) => Some(error),
        }
    }

    // LEARNING: `&Assistant` and `&str` borrow existing values; the session does not own the assistant
    // or prompt.
    // The owned history sent to Rig contains prior successful native batches; the current prompt is
    // passed separately once.
    pub(crate) async fn respond(
        &mut self,
        assistant: &Assistant,
        prompt: &str,
    ) -> ConversationTurn {
        let history = self.history();
        let run = assistant.respond(prompt, &history).await;

        // LEARNING: Matching on `&run` inspects the typed Result through references, so `run` remains
        // intact to return with its report.
        // Only success updates active history. The archive write follows inference, so a save failure
        // cannot cause another model run.
        let (outcome, context_warning) = match &run {
            Ok(run) => {
                let context_warning = self.record_success(run.response.messages());
                (
                    ArchiveOutcome::Completed(run.response.output().to_owned()),
                    context_warning,
                )
            }
            Err(error) => (archive_outcome(error), None),
        };

        // LEARNING: `as_ref()` borrows the archive handle. `.err()` turns the append Result into an
        // optional storage error.
        // Callers can report this sidecar error without replacing or retrying the model outcome.
        let archive_error = match self.archive.as_ref() {
            Some(archive) => archive.append_turn(prompt, outcome).await.err(),
            None => None,
        };

        ConversationTurn {
            run,
            context_warning,
            archive_error,
        }
    }

    fn history(&self) -> Vec<Message> {
        // LEARNING: This resembles Java Streams `flatMap` and collection, but `.cloned().collect()`
        // builds an owned `Vec<Message>`.
        // Here `cloned()` copies each Message value rather than a Java-style shared object reference.
        self.batches.iter().flatten().cloned().collect()
    }

    fn record_success(&mut self, messages: Option<&[Message]>) -> Option<&'static str> {
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

        self.batches.push_back(messages.to_vec());
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

// LEARNING: This exhaustive match maps detailed run errors to safe archive categories; raw provider
// diagnostics remain in the run and are not archived.
fn archive_outcome(error: &AssistantRunError) -> ArchiveOutcome {
    match &error.error {
        AssistantError::Prompt(_) => ArchiveOutcome::PromptFailed,
        AssistantError::RunTimeout => ArchiveOutcome::TimedOut,
        AssistantError::InvalidResponse => ArchiveOutcome::InvalidResponse,
    }
}

// LEARNING: Borrowed messages are serialized for an exact JSON byte count. The `Result` makes
// serialization failure explicit to the caller.
fn serialized_batch_bytes(messages: &[Message]) -> Result<usize, serde_json::Error> {
    serde_json::to_vec(messages).map(|serialized| serialized.len())
}

// LEARNING: `Vec<&Message>` is a temporary flattened view for counting.
// Sizing history does not clone the messages again.
fn serialized_history_bytes(batches: &VecDeque<Vec<Message>>) -> Result<usize, serde_json::Error> {
    let history: Vec<&Message> = batches.iter().flatten().collect();
    serde_json::to_vec(&history).map(|serialized| serialized.len())
}

#[cfg(test)]
#[path = "../../tests/unit/services/conversation_tests.rs"]
mod tests;
