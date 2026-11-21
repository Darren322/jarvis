use std::{
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use rig_agent::AgentBuilder;
use rig_core::{
    completion::Usage,
    message::{Message, UserContent},
    test_utils::{MockCompletionModel, MockStreamEvent},
};
use rusqlite::Connection;
use tokio::sync::mpsc;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

use crate::{
    clients::remote_embeddings::RemoteEmbeddings,
    config::EmbeddingConfig,
    features::{
        assistant::Assistant,
        memory::{MemoryService, RecallMode, facts_message},
    },
    storage::{ArchiveOutcome, FactOrigin, MemoryEligibility, MemorySaveState},
};

use super::{ConversationSession, ConversationTurn, merge_source_dependencies};

impl ConversationSession {
    /// Drives the production `respond_stream` path with no memory and no search,
    /// draining visible deltas concurrently as the CLI would.
    async fn respond(&mut self, assistant: &Assistant, prompt: &str) -> ConversationTurn {
        self.respond_in_mode(assistant, None, RecallMode::Skip, prompt)
            .await
    }

    /// Same, with `mode` deciding whether `memory` is searched.
    async fn respond_with_memory(
        &mut self,
        assistant: &Assistant,
        memory: &MemoryService,
        mode: RecallMode,
        prompt: &str,
    ) -> ConversationTurn {
        self.respond_in_mode(assistant, Some(memory), mode, prompt)
            .await
    }

    async fn respond_in_mode(
        &mut self,
        assistant: &Assistant,
        memory: Option<&MemoryService>,
        mode: RecallMode,
        prompt: &str,
    ) -> ConversationTurn {
        let (deltas, mut receiver) = mpsc::channel(16);
        let drain = async { while receiver.recv().await.is_some() {} };
        let (turn, ()) = tokio::join!(
            self.respond_stream(assistant, memory, mode, prompt, deltas),
            drain
        );
        turn
    }
}

fn answer(text: &str) -> Vec<MockStreamEvent> {
    vec![
        MockStreamEvent::text(text),
        MockStreamEvent::final_response(Usage::new()),
    ]
}

fn failure(message: &str) -> Vec<MockStreamEvent> {
    vec![MockStreamEvent::error(message)]
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
    let model = MockCompletionModel::from_stream_turns([
        answer("first answer"),
        failure("temporary provider failure"),
        answer("third answer"),
    ]);
    let model_handle = model.clone();
    let assistant = Assistant::new(
        crate::tools::register(
            AgentBuilder::new(model),
            crate::features::panel::channel().0,
        )
        .build(),
    );
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

    let failed = session.respond(&assistant, "failed prompt").await;
    assert!(failed.run.is_err());
    assert_eq!(session.history.stored_messages(), first_messages);

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
    let model = MockCompletionModel::from_stream_turns([
        answer("saved answer"),
        answer("new session answer"),
    ]);
    let model_handle = model.clone();
    let assistant = Assistant::new(
        crate::tools::register(
            AgentBuilder::new(model),
            crate::features::panel::channel().0,
        )
        .build(),
    );
    let mut session = ConversationSession::new(path.clone());

    assert!(session.reset().await.is_ok());
    let turn = session.respond(&assistant, "saved prompt").await;
    assert!(turn.run.is_ok());
    assert!(turn.archive_error.is_none());
    assert!(!session.history.stored_messages().is_empty());

    assert!(session.reset().await.is_ok());
    assert!(session.history.stored_messages().is_empty());
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
    let model =
        MockCompletionModel::from_stream_turns([answer("visible answer"), answer("next answer")]);
    let model_handle = model.clone();
    let assistant = Assistant::new(
        crate::tools::register(
            AgentBuilder::new(model),
            crate::features::panel::channel().0,
        )
        .build(),
    );
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

/// A session whose archive holds "My cat is called Miso." as searchable memory,
/// with memory opened against an embedding server that always refuses (recall is
/// then keyword-only, which is enough here). The server is returned so tests can
/// count the requests it received.
async fn session_with_cat_memory() -> (ConversationSession, PathBuf, MemoryService, MockServer) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/embeddings"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;
    let embeddings = RemoteEmbeddings::new(&EmbeddingConfig {
        base_url: format!("{}/v1", server.uri()),
        model: "bge-test".to_owned(),
    })
    .expect("a localhost URL is allowed");
    let path = archive_path();
    let mut session = ConversationSession::new(path.clone());
    assert!(session.reset().await.is_ok());
    session
        .archive
        .as_ref()
        .expect("archive is open")
        .append_turn(
            "My cat is called Miso.",
            ArchiveOutcome::Completed("Noted, master.".to_owned()),
            MemoryEligibility::Eligible,
            &[],
        )
        .await
        .expect("remembered turn archives");
    let memory = MemoryService::open(
        session.memory_repository().expect("archive is open"),
        embeddings,
    );
    memory.wait_until_embedder_idle().await;
    (session, path, memory, server)
}

/// How many embedding requests the server has received.
async fn embedding_requests(server: &MockServer) -> usize {
    server
        .received_requests()
        .await
        .expect("request recording is on")
        .iter()
        .filter(|request| request.url.path() == "/v1/embeddings")
        .count()
}

#[tokio::test]
async fn recalled_memory_is_sent_with_the_run_but_never_stored_in_history() {
    let model = MockCompletionModel::from_stream_turns([
        answer("Your cat has a name."),
        answer("No further notes."),
    ]);
    let model_handle = model.clone();
    let assistant = Assistant::new(
        crate::tools::register(
            AgentBuilder::new(model),
            crate::features::panel::channel().0,
        )
        .build(),
    );
    let (mut session, path, memory, server) = session_with_cat_memory().await;

    let first = session
        .respond_with_memory(
            &assistant,
            &memory,
            RecallMode::Search,
            "What is my cat called?",
        )
        .await;
    let first_batch = first
        .run
        .as_ref()
        .expect("first response succeeds")
        .response
        .messages()
        .expect("Rig should return the successful batch")
        .to_vec();
    let second = session
        .respond_with_memory(&assistant, &memory, RecallMode::Skip, "Anything else?")
        .await;
    assert!(second.run.is_ok());

    let requests = model_handle.requests();
    assert_eq!(requests.len(), 2);
    // With no history yet, the first request carried the memory directly
    // before the prompt (with history it would sit before the last exchange)...
    assert_eq!(requests[0].chat_history.len(), 2);
    assert!(
        format!("{:?}", requests[0].chat_history[0]).contains("Miso"),
        "recalled memory should reach the model"
    );
    assert_eq!(
        requests[0].chat_history[1],
        user_message("What is my cat called?")
    );
    // ...but the stored batch is only the prompt and the answer, so the second
    // request starts with exactly that batch and no memory message.
    assert_eq!(first_batch.len(), 2);
    assert_eq!(first_batch[0], user_message("What is my cat called?"));
    assert!(!format!("{first_batch:?}").contains("Miso"));
    assert_eq!(
        requests[1].chat_history[..first_batch.len()],
        first_batch[..]
    );
    assert_eq!(
        requests[1].chat_history.last(),
        Some(&user_message("Anything else?"))
    );
    assert!(!format!("{:?}", requests[1].chat_history).contains("Miso"));
    assert!(!format!("{:?}", session.history.stored_messages()).contains("Miso"));

    // Only the searching run asked the embedding server for anything.
    assert_eq!(embedding_requests(&server).await, 1);

    drop(memory);
    drop(session);
    remove_archive(&path);
}

#[tokio::test]
async fn a_run_without_a_search_sends_no_memory_and_no_warning_but_is_still_stored() {
    let model = MockCompletionModel::from_stream_turns([answer("It is Miso.")]);
    let model_handle = model.clone();
    let assistant = Assistant::new(
        crate::tools::register(
            AgentBuilder::new(model),
            crate::features::panel::channel().0,
        )
        .build(),
    );
    let (mut session, path, memory, server) = session_with_cat_memory().await;

    let turn = session
        .respond_with_memory(
            &assistant,
            &memory,
            RecallMode::Skip,
            "What is my cat called?",
        )
        .await;

    assert!(turn.run.is_ok());
    // Matching memory exists, but it was not asked for: not in the request,
    // no embedding request, no warning.
    let requests = model_handle.requests();
    assert_eq!(
        requests[0].chat_history,
        vec![user_message("What is my cat called?")]
    );
    assert_eq!(embedding_requests(&server).await, 0);
    assert_eq!(turn.context_warning, None);
    // The turn is archived, indexed (it has a source) and recorded in the history.
    assert!(turn.archive_error.is_none());
    assert_eq!(turn.memory_state, Some(MemorySaveState::Saved));
    assert!(turn.current_source_id.is_some());
    assert!(!session.history.stored_messages().is_empty());

    drop(memory);
    drop(session);
    let connection = Connection::open(&path).expect("archive database opens");
    let turns: i64 = connection
        .query_row("SELECT COUNT(*) FROM turns", [], |row| row.get(0))
        .expect("turn count is readable");
    assert_eq!(turns, 2);
    drop(connection);
    remove_archive(&path);
}

#[tokio::test]
async fn only_a_search_that_cannot_run_warns_that_memory_is_unavailable() {
    let model = MockCompletionModel::from_stream_turns([answer("one"), answer("two")]);
    let assistant = Assistant::new(
        crate::tools::register(
            AgentBuilder::new(model),
            crate::features::panel::channel().0,
        )
        .build(),
    );
    let path = archive_path();
    let mut session = ConversationSession::new(path.clone());
    assert!(session.reset().await.is_ok());

    let skipped = session
        .respond_in_mode(&assistant, None, RecallMode::Skip, "Hello")
        .await;
    let searched = session
        .respond_in_mode(&assistant, None, RecallMode::Search, "Do you remember me?")
        .await;

    assert_eq!(skipped.context_warning, None);
    assert!(
        searched
            .context_warning
            .is_some_and(|warning| warning.contains("memory storage is unavailable"))
    );

    drop(session);
    remove_archive(&path);
}

#[tokio::test]
async fn requests_between_trims_share_their_history_and_a_trim_hands_back_the_warm_up() {
    // `MAX_EXCHANGES` and `TRIMMED_EXCHANGES` in `history.rs`.
    const UPPER: usize = 12;
    const LOWER: usize = 6;
    let trimming_turn = UPPER + 1;
    // One scripted answer per turn, one for the warm-up, and one turn after it.
    let model = MockCompletionModel::from_stream_turns(
        (1..=trimming_turn + 2).map(|number| answer(&format!("answer {number}"))),
    );
    let model_handle = model.clone();
    let assistant = Assistant::new(
        crate::tools::register(
            AgentBuilder::new(model),
            crate::features::panel::channel().0,
        )
        .build(),
    );
    let mut session = ConversationSession::new(archive_path());

    for number in 1..=trimming_turn {
        let turn = session
            .respond(&assistant, &format!("question {number}"))
            .await;
        assert!(turn.run.is_ok());
        assert_eq!(
            turn.cache_warmup.is_some(),
            number == trimming_turn,
            "turn {number}"
        );
        if let Some(warmup) = turn.cache_warmup {
            assistant
                .warm_cache(warmup)
                .await
                .expect("the scripted warm-up succeeds");
        }
    }
    let after_trim = session.respond(&assistant, "question after").await;
    assert!(after_trim.run.is_ok());
    assert!(after_trim.cache_warmup.is_none());

    let requests = model_handle.requests();
    assert_eq!(requests.len(), trimming_turn + 2);
    // Up to the trim, each request is the one before it plus that turn's answer and
    // the new prompt: the model server's cached prompt stays valid.
    for pair in requests[..trimming_turn].windows(2) {
        let earlier = &pair[0].chat_history;
        assert_eq!(pair[1].chat_history.len(), earlier.len() + 2);
        assert_eq!(pair[1].chat_history[..earlier.len()], earlier[..]);
    }
    // The warm-up is the start of the first request after the trim, short of the
    // last answer: the kept exchanges, ending on the last one's prompt.
    let warm = &requests[trimming_turn].chat_history;
    let next = &requests[trimming_turn + 1].chat_history;
    assert_eq!(warm.len(), 2 * LOWER - 1);
    assert_eq!(warm.first(), Some(&user_message("question 8")));
    assert_eq!(warm.last(), Some(&user_message("question 13")));
    assert_eq!(next.len(), 2 * LOWER + 1);
    assert_eq!(next[..warm.len()], warm[..]);
    assert_eq!(next.last(), Some(&user_message("question after")));
}

fn mock_assistant(turns: usize) -> (Assistant, MockCompletionModel) {
    let model = MockCompletionModel::from_stream_turns(
        (1..=turns).map(|number| answer(&format!("answer {number}"))),
    );
    let handle = model.clone();
    let assistant = Assistant::new(
        crate::tools::register(
            AgentBuilder::new(model),
            crate::features::panel::channel().0,
        )
        .build(),
    );
    (assistant, handle)
}

/// How many messages of a request are the facts message.
fn facts_messages_in(history: &[Message]) -> usize {
    history
        .iter()
        .filter(|message| format!("{message:?}").contains("Saved facts about the user"))
        .count()
}

#[tokio::test]
async fn every_request_reads_the_facts_list_so_changes_show_in_the_next_one() {
    let (assistant, model) = mock_assistant(4);
    let path = archive_path();
    let mut session = ConversationSession::new(path.clone());
    assert!(session.reset().await.is_ok());
    let facts = session.facts_repository().expect("archive is open");

    let empty = session.respond(&assistant, "question 1").await;
    let sister = facts
        .insert("My sister is called Mia.", FactOrigin::Stated)
        .await
        .expect("fact is added");
    facts
        .insert("I wake at six.", FactOrigin::Noted)
        .await
        .expect("fact is added");
    let listed = facts.list().await.expect("facts list");
    let both = session.respond(&assistant, "question 2").await;
    facts.remove(sister.id).await.expect("fact is removed");
    let one = session.respond(&assistant, "question 3").await;
    let remaining = facts.list().await.expect("facts list");
    facts
        .remove(remaining[0].id)
        .await
        .expect("fact is removed");
    let none = session.respond(&assistant, "question 4").await;

    for turn in [&empty, &both, &one, &none] {
        assert!(turn.run.is_ok());
        assert_eq!(turn.context_warning, None);
    }
    let requests = model.requests();
    let first_message = |index: usize| requests[index].chat_history.first().cloned();
    // No facts: no message, and the conversation starts with the first prompt.
    assert_eq!(first_message(0), Some(user_message("question 1")));
    // Facts added after the first turn are in the second request, in id order.
    let second = requests[1].chat_history.to_vec();
    assert_eq!(listed.len(), 2);
    assert_eq!(facts_messages_in(&second), 1);
    assert_eq!(Some(second[0].clone()), facts_message(&listed));
    assert!(
        format!("{:?}", second[0]).find("Mia") < format!("{:?}", second[0]).find("wake"),
        "oldest first"
    );
    // A removed fact is absent from the very next request.
    let third = requests[2].chat_history.to_vec();
    assert_eq!(facts_messages_in(&third), 1);
    assert!(!format!("{:?}", third[0]).contains("Mia"));
    assert!(format!("{:?}", third[0]).contains("I wake at six."));
    // Once the list is empty again no message is sent, and none was ever stored
    // in the conversation: the history in the last request holds only exchanges.
    let fourth = requests[3].chat_history.to_vec();
    assert_eq!(facts_messages_in(&fourth), 0);
    assert_eq!(fourth.first(), Some(&user_message("question 1")));
    assert_eq!(fourth.len(), 7);

    drop(session);
    remove_archive(&path);
}

#[tokio::test]
async fn a_failed_facts_read_warns_and_the_turn_goes_out_without_facts() {
    let (assistant, model) = mock_assistant(1);
    let path = archive_path();
    let mut session = ConversationSession::new(path.clone());
    assert!(session.reset().await.is_ok());
    // Break the table from outside, as a damaged database would.
    Connection::open(&path)
        .expect("archive database opens")
        .execute_batch("DROP TABLE facts;")
        .expect("facts table is dropped");

    let turn = session.respond(&assistant, "question 1").await;

    assert!(turn.run.is_ok(), "a facts failure never fails the turn");
    assert!(
        turn.context_warning
            .is_some_and(|warning| warning.contains("facts list is unavailable"))
    );
    assert_eq!(
        model.requests()[0].chat_history.to_vec(),
        [user_message("question 1")]
    );

    drop(session);
    remove_archive(&path);
}

#[tokio::test]
async fn a_trim_warm_up_carries_the_facts_the_next_request_starts_with() {
    const UPPER: usize = 12;
    let trimming_turn = UPPER + 1;
    let (assistant, model) = mock_assistant(trimming_turn + 2);
    let path = archive_path();
    let mut session = ConversationSession::new(path.clone());
    assert!(session.reset().await.is_ok());
    session
        .facts_repository()
        .expect("archive is open")
        .insert("My sister is called Mia.", FactOrigin::Stated)
        .await
        .expect("fact is added");

    for number in 1..=trimming_turn {
        let turn = session
            .respond(&assistant, &format!("question {number}"))
            .await;
        assert!(turn.run.is_ok());
        if let Some(warmup) = turn.cache_warmup {
            assistant
                .warm_cache(warmup)
                .await
                .expect("the scripted warm-up succeeds");
        }
    }
    let after_trim = session.respond(&assistant, "question after").await;
    assert!(after_trim.run.is_ok());

    let requests = model.requests();
    let warm = requests[trimming_turn].chat_history.to_vec();
    let next = requests[trimming_turn + 1].chat_history.to_vec();
    assert_eq!(facts_messages_in(&warm), 1);
    assert!(format!("{:?}", warm[0]).contains("My sister is called Mia."));
    assert_eq!(
        next[..warm.len()],
        warm[..],
        "the warm-up is byte-identical to the start of the next request, facts included"
    );
    // Before the trim, every request began with the same facts message.
    for request in &requests[..trimming_turn] {
        assert_eq!(request.chat_history[0], warm[0]);
    }

    drop(session);
    remove_archive(&path);
}
