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

use crate::{
    features::assistant::Assistant,
    storage::{ArchiveOutcome, MemoryEligibility},
    tools::system_status_tool::SystemStatusTool,
};

use super::{ConversationSession, ConversationTurn, archive_outcome, merge_source_dependencies};

impl ConversationSession {
    // LEARNING: `&Assistant` and `&str` borrow existing values; the session does not own the assistant
    // or prompt.
    // The owned history sent to Rig contains prior successful native batches; the current prompt is
    // passed separately once.
    #[cfg(test)]
    pub(crate) async fn respond(
        &mut self,
        assistant: &Assistant,
        prompt: &str,
    ) -> ConversationTurn {
        let history = self.history.messages();
        let run = assistant.respond(prompt, &history).await;

        // LEARNING: Matching on `&run` inspects the typed Result through references, so `run` remains
        // intact to return with its report.
        // Only success updates active history. The archive write follows inference, so a save failure
        // cannot cause another model run.
        let (outcome, context_warning) = match &run {
            Ok(run) => {
                let context_warning = self
                    .history
                    .record_success(run.response.messages(), &[], None)
                    .map(str::to_owned);
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
            Some(archive) => archive
                .append_turn(prompt, outcome, MemoryEligibility::ArchiveOnly, &[])
                .await
                .err(),
            None => None,
        };

        ConversationTurn {
            run,
            context_warning,
            archive_error,
            memory_state: None,
            current_source_id: None,
        }
    }
}

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

#[tokio::test]
async fn history_only_answer_keeps_recalled_source_lineage_for_forget_cascade() {
    let path = archive_path();
    let mut session = ConversationSession::new(path.clone());
    assert!(session.reset().await.is_ok());
    let archive = session.archive.as_ref().expect("archive is open");
    let repository = archive.memory_repository();
    let recalled = repository
        .import_source(
            "/notes/fact.txt".to_owned(),
            "fact.txt".to_owned(),
            "The workshop is on Tuesday.".to_owned(),
        )
        .await
        .expect("source should import");

    let answer_b = archive
        .append_turn(
            "When is the workshop?",
            ArchiveOutcome::Completed("It is on Tuesday.".to_owned()),
            MemoryEligibility::Eligible,
            &[recalled.source_id],
        )
        .await
        .expect("answer B should archive");
    let answer_b_source = answer_b.source_id.expect("eligible answer has a source");

    // Turn C only needs native history at inference time. Its archive lineage must
    // still include the recalled source that informed turn B, or forgetting that
    // source could leave C searchable.
    let dependencies = merge_source_dependencies(&[recalled.source_id, answer_b_source], &[]);
    let answer_c = archive
        .append_turn(
            "And when is that?",
            ArchiveOutcome::Completed("Tuesday.".to_owned()),
            MemoryEligibility::Eligible,
            &dependencies,
        )
        .await
        .expect("answer C should archive");
    let answer_c_source = answer_c.source_id.expect("eligible answer has a source");
    repository
        .forget_source(recalled.source_id)
        .await
        .expect("forget should suppress the source lineage");

    drop(repository);
    drop(session);
    let connection = Connection::open(&path).expect("archive database opens");
    let child_depends_on_recalled: bool = connection
        .query_row(
            "SELECT EXISTS(
                SELECT 1 FROM source_links
                WHERE source_id = ?1 AND depends_on_source_id = ?2
            )",
            rusqlite::params![answer_c_source.0, recalled.source_id.0],
            |row| row.get(0),
        )
        .expect("C lineage is queryable");
    let child_is_suppressed: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sources WHERE id = ?1 AND state = 'forgotten')",
            [answer_c_source.0],
            |row| row.get(0),
        )
        .expect("C suppression is queryable");
    assert!(child_depends_on_recalled);
    assert!(child_is_suppressed);

    drop(connection);
    remove_archive(&path);
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
    assert!(session.reset().await.is_ok());

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
    assert_eq!(session.history.messages(), first_messages);

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

    assert!(session.reset().await.is_ok());
    let turn = session.respond(&assistant, "saved prompt").await;
    assert!(turn.run.is_ok());
    assert!(turn.archive_error.is_none());
    assert!(!session.history.messages().is_empty());

    assert!(session.reset().await.is_ok());
    assert!(session.history.messages().is_empty());
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
    assert!(session.reset().await.is_ok());

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
