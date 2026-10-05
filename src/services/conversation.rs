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
mod tests {
    use std::{
        path::PathBuf,
        sync::atomic::{AtomicUsize, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    use rig_agent::AgentBuilder;
    use rig_core::{
        message::{Message, UserContent},
        test_utils::{MockCompletionModel, MockTurn},
    };
    use rusqlite::Connection;

    use crate::{services::assistant::Assistant, tools::system_status_tool::SystemStatusTool};

    use super::{ConversationSession, MAX_HISTORY_BYTES, serialized_batch_bytes};

    static NEXT_PATH: AtomicUsize = AtomicUsize::new(0);

    fn user_message(text: &str) -> Message {
        Message::User {
            content: vec![UserContent::text(text)],
        }
    }

    fn archive_path() -> PathBuf {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after the Unix epoch")
            .as_nanos();
        let suffix = NEXT_PATH.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "jarvis-conversation-{}-{timestamp}-{suffix}.sqlite3",
            std::process::id()
        ))
    }

    fn remove_archive(path: &std::path::Path) {
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn keeps_three_whole_successful_batches() {
        let mut session = ConversationSession::new(archive_path());
        let batch = |prompt: &str, answer: &str| {
            vec![
                user_message(prompt),
                Message::Assistant {
                    id: None,
                    content: vec![rig_core::message::AssistantContent::text(answer)],
                },
            ]
        };

        let first = batch("one", "answer one");
        let second = batch("two", "answer two");
        let third = batch("three", "answer three");
        let fourth = batch("four", "answer four");

        session.record_success(Some(&first));
        session.record_success(Some(&second));
        session.record_success(Some(&third));
        session.record_success(Some(&fourth));

        assert_eq!(session.history(), [second, third, fourth].concat());
    }

    #[test]
    fn history_byte_limit_keeps_exact_boundary_and_evicts_oldest_batch() {
        let mut session = ConversationSession::new(archive_path());
        let empty_message = [user_message("")];
        let overhead = serialized_batch_bytes(&empty_message).expect("message serializes");
        let exact_batch = [user_message(&"x".repeat(MAX_HISTORY_BYTES - overhead))];

        assert_eq!(
            serialized_batch_bytes(&exact_batch).expect("message serializes"),
            MAX_HISTORY_BYTES
        );
        assert_eq!(session.record_success(Some(&exact_batch)), None);
        assert_eq!(session.history(), exact_batch.to_vec());

        let newest = [user_message("newest")];
        assert_eq!(session.record_success(Some(&newest)), None);
        assert_eq!(session.history(), newest.to_vec());
    }

    #[test]
    fn oversized_new_batch_clears_older_context_and_is_not_retained() {
        let mut session = ConversationSession::new(archive_path());
        let prior = [user_message("prior")];
        session.record_success(Some(&prior));

        let overhead = serialized_batch_bytes(&[user_message("")]).expect("message serializes");
        let oversized = [user_message(&"x".repeat(MAX_HISTORY_BYTES + 1 - overhead))];

        assert!(session.record_success(Some(&oversized)).is_some());
        assert!(session.history().is_empty());
    }

    #[test]
    fn missing_messages_clears_active_history_and_warns() {
        let mut session = ConversationSession::new(archive_path());
        session.record_success(Some(&[user_message("prior")]));

        assert!(session.record_success(None).is_some());
        assert!(session.history().is_empty());
    }

    #[tokio::test]
    async fn history_reaches_next_request_once_and_failed_prompt_is_not_retained() {
        let model = MockCompletionModel::from_turns([
            MockTurn::text("first answer"),
            MockTurn::error("temporary provider failure"),
            MockTurn::text("third answer"),
        ]);
        let model_handle = model.clone();
        let assistant = Assistant::new(AgentBuilder::new(model).tool(SystemStatusTool).build());
        let path = archive_path();
        let mut session = ConversationSession::new(path.clone());
        assert!(session.reset().await.is_none());

        let first = session.respond(&assistant, "first prompt").await;
        let first_messages = first
            .run
            .as_ref()
            .expect("first response succeeds")
            .response
            .messages()
            .expect("Rig should return the successful batch")
            .to_vec();
        assert!(first.context_warning.is_none());

        let failed = session.respond(&assistant, "failed prompt").await;
        assert!(failed.run.is_err());
        assert_eq!(session.history(), first_messages);

        let third = session.respond(&assistant, "third prompt").await;
        assert!(third.run.is_ok());
        let requests = model_handle.requests();
        assert_eq!(model_handle.request_count(), 3);
        assert_eq!(requests[2].chat_history.len(), first_messages.len() + 1);
        assert_eq!(
            &requests[2].chat_history[..first_messages.len()],
            first_messages.as_slice()
        );
        assert_eq!(
            requests[2].chat_history.last(),
            Some(&user_message("third prompt"))
        );

        drop(session);
        let connection = Connection::open(&path).expect("archive database opens");
        let mut statement = connection
            .prepare("SELECT user_text, assistant_text, outcome FROM turns ORDER BY id")
            .expect("turn query prepares");
        let archived_turns = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .expect("turn query runs")
            .collect::<Result<Vec<_>, _>>()
            .expect("turn rows are readable");
        assert_eq!(
            archived_turns,
            vec![
                (
                    "first prompt".to_owned(),
                    Some("first answer".to_owned()),
                    "completed".to_owned(),
                ),
                ("failed prompt".to_owned(), None, "prompt_failed".to_owned(),),
                (
                    "third prompt".to_owned(),
                    Some("third answer".to_owned()),
                    "completed".to_owned(),
                ),
            ]
        );
        assert!(archived_turns.iter().all(|(user, answer, outcome)| {
            let assistant_text_is_safe = match answer {
                Some(answer) => !answer.contains("temporary provider failure"),
                None => true,
            };
            !user.contains("temporary provider failure")
                && assistant_text_is_safe
                && !outcome.contains("temporary provider failure")
        }));

        drop(statement);
        drop(connection);
        remove_archive(&path);
    }

    #[tokio::test]
    async fn reset_starts_fresh_context_and_preserves_archive_rows() {
        let path = archive_path();
        let model = MockCompletionModel::from_turns([
            MockTurn::text("saved answer"),
            MockTurn::text("new session answer"),
        ]);
        let model_handle = model.clone();
        let assistant = Assistant::new(AgentBuilder::new(model).tool(SystemStatusTool).build());
        let mut session = ConversationSession::new(path.clone());

        assert!(session.reset().await.is_none());
        let turn = session.respond(&assistant, "saved prompt").await;
        assert!(turn.run.is_ok());
        assert!(turn.archive_error.is_none());
        assert!(!session.history().is_empty());

        assert!(session.reset().await.is_none());
        assert!(session.history().is_empty());
        let next = session.respond(&assistant, "new session prompt").await;
        assert!(next.run.is_ok());
        drop(session);

        let requests = model_handle.requests();
        assert_eq!(model_handle.request_count(), 2);
        assert_eq!(
            requests[1].chat_history,
            vec![user_message("new session prompt")]
        );

        let connection = Connection::open(&path).expect("archive database opens");
        let session_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))
            .expect("session count is readable");
        let turn_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM turns", [], |row| row.get(0))
            .expect("turn count is readable");
        assert_eq!(session_count, 2);
        assert_eq!(turn_count, 2);

        drop(connection);
        remove_archive(&path);
    }

    #[tokio::test]
    async fn archive_write_failure_keeps_model_result_and_allows_the_next_run() {
        let path = archive_path();
        let model = MockCompletionModel::from_turns([
            MockTurn::text("visible answer"),
            MockTurn::text("next answer"),
        ]);
        let model_handle = model.clone();
        let assistant = Assistant::new(AgentBuilder::new(model).tool(SystemStatusTool).build());
        let mut session = ConversationSession::new(path.clone());
        assert!(session.reset().await.is_none());

        let connection = Connection::open(&path).expect("archive database opens");
        connection
            .execute_batch(
                "CREATE TRIGGER fail_test_turn BEFORE INSERT ON turns \
                 BEGIN SELECT RAISE(ABORT, 'simulated archive write failure'); END;",
            )
            .expect("test trigger is created");
        drop(connection);

        let turn = session.respond(&assistant, "write will fail").await;
        let first_messages = turn
            .run
            .as_ref()
            .expect("model response remains successful")
            .response
            .messages()
            .expect("Rig should return the successful batch")
            .to_vec();
        assert_eq!(
            turn.run
                .as_ref()
                .expect("model response remains successful")
                .response
                .output(),
            "visible answer"
        );
        assert!(turn.archive_error.is_some());
        assert_eq!(model_handle.request_count(), 1);

        let connection = Connection::open(&path).expect("archive database opens");
        connection
            .execute_batch("DROP TRIGGER fail_test_turn;")
            .expect("test trigger is removed");
        drop(connection);

        let next = session.respond(&assistant, "after archive failure").await;
        assert!(next.run.is_ok());
        assert!(next.archive_error.is_none());
        let requests = model_handle.requests();
        assert_eq!(model_handle.request_count(), 2);
        assert_eq!(requests[1].chat_history.len(), first_messages.len() + 1);
        assert_eq!(
            &requests[1].chat_history[..first_messages.len()],
            first_messages.as_slice()
        );
        assert_eq!(
            requests[1].chat_history.last(),
            Some(&user_message("after archive failure"))
        );

        drop(session);
        let connection = Connection::open(&path).expect("archive database opens");
        let saved_turn_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM turns", [], |row| row.get(0))
            .expect("saved turn count is readable");
        assert_eq!(saved_turn_count, 1);
        drop(connection);
        remove_archive(&path);
    }
}
