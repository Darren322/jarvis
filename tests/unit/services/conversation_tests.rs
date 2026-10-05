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
