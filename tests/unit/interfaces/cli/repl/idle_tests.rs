use std::{io, sync::Arc};

use rig_agent::prelude::{CompletionError, CompletionModel};
use rig_core::{
    completion::{CompletionRequest, CompletionResponse, Usage},
    streaming::StreamingCompletionResponse,
    test_utils::{MockCompletionModel, MockStreamEvent},
};
use tempfile::TempDir;
use tokio::{
    io::{AsyncWriteExt, BufReader, DuplexStream},
    sync::{Notify, mpsc},
};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

use crate::{
    clients::remote_embeddings::RemoteEmbeddings,
    config::EmbeddingConfig,
    features::{
        assistant::FactExtractor,
        memory::{FactNoter, MemoryService},
        state::AssistantState,
    },
    storage::{
        ArchiveOutcome, ConversationArchive, Examination, Fact, FactId, FactOrigin,
        MemoryEligibility,
    },
};

use super::super::super::{
    input::{InputLine, InputReader},
    presentation::AssistantPresenter,
};
use super::{read_line_during_idle_work, show_noted};

/// Collects the notices the idle read shows, and tells the test about each as it comes.
struct Notices(mpsc::UnboundedSender<String>);

impl AssistantPresenter for Notices {
    fn delta(&mut self, _text: &str) -> io::Result<()> {
        Ok(())
    }

    fn completed(&mut self, _canonical_answer: &str) -> io::Result<()> {
        Ok(())
    }

    fn failed(&mut self, _safe_message: &str) -> io::Result<()> {
        Ok(())
    }

    fn prompt(&mut self) -> io::Result<()> {
        Ok(())
    }

    fn notice(&mut self, line: &str) -> io::Result<()> {
        let _ = self.0.send(line.to_owned());
        Ok(())
    }

    fn state(&mut self, _state: AssistantState) -> io::Result<()> {
        Ok(())
    }

    fn input_received(&mut self) {}
}

/// Says when a request reaches it, then never answers.
#[derive(Clone, Default)]
struct SignallingModel {
    started: Arc<Notify>,
}

impl CompletionModel for SignallingModel {
    async fn completion(
        &self,
        _request: CompletionRequest,
    ) -> Result<CompletionResponse, CompletionError> {
        unreachable!("the extraction streams")
    }

    async fn stream(
        &self,
        _request: CompletionRequest,
    ) -> Result<StreamingCompletionResponse, CompletionError> {
        self.started.notify_one();
        std::future::pending().await
    }
}

fn answer(text: &str) -> Vec<MockStreamEvent> {
    vec![
        MockStreamEvent::text(text),
        MockStreamEvent::final_response(Usage::new()),
    ]
}

/// A database with one completed turn, and an embedding server that is down, so the
/// turn's memory waits for a vector that cannot come.
struct Harness {
    _directory: TempDir,
    archive: ConversationArchive,
    _server: MockServer,
    embeddings: EmbeddingConfig,
}

impl Harness {
    async fn open(said: &str) -> Self {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let archive = ConversationArchive::open(directory.path().join("jarvis.sqlite3"))
            .await
            .expect("archive opens");
        archive
            .append_turn(
                said,
                ArchiveOutcome::Completed("ANSWER-JARVIS-GAVE".to_owned()),
                MemoryEligibility::Eligible,
                &[],
            )
            .await
            .expect("turn is archived");
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/health"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        let embeddings = EmbeddingConfig {
            base_url: format!("{}/v1", server.uri()),
            model: "bge-test".to_owned(),
        };
        Self {
            _directory: directory,
            archive,
            _server: server,
            embeddings,
        }
    }

    /// Memory over this database whose idle time notes facts through `model`.
    async fn memory(&self, model: impl CompletionModel + 'static) -> MemoryService {
        let memory = MemoryService::open(
            self.archive.memory_repository(),
            RemoteEmbeddings::new(&self.embeddings).expect("a localhost URL is allowed"),
        )
        .with_fact_noting(FactNoter::new(
            FactExtractor::new(model),
            self.archive.facts_repository(),
        ));
        memory.wait_until_embedder_idle().await;
        memory
    }

    async fn turn_is_waiting(&self) -> bool {
        self.archive
            .facts_repository()
            .next_unexamined_turn(&[])
            .await
            .expect("next turn is read")
            .is_some()
    }

    async fn fact_texts(&self) -> Vec<String> {
        self.archive
            .facts_repository()
            .list()
            .await
            .expect("facts list")
            .into_iter()
            .map(|fact| fact.text)
            .collect()
    }
}

/// An input the test types into, and the reader over it.
fn typed_input() -> (DuplexStream, InputReader<BufReader<DuplexStream>>) {
    let (typing, reader) = tokio::io::duplex(256);
    (typing, InputReader::new(BufReader::new(reader)))
}

fn assert_line(line: &InputLine, expected: &str) {
    assert!(
        matches!(line, InputLine::Prompt(text) if text == expected),
        "expected the typed line {expected:?}"
    );
}

/// Lets the other tasks and the SQLite worker run a while: for a test that needs
/// "nothing else happens" without a wall-clock sleep.
async fn let_others_run() {
    for _ in 0..200 {
        tokio::task::yield_now().await;
    }
}

#[tokio::test]
async fn a_fact_noted_while_idle_is_shown_at_once_stored_and_the_turn_examined() {
    let harness = Harness::open("My cat is called Miso.").await;
    let model =
        MockCompletionModel::from_stream_turns([answer("FACT: The user's cat is called Miso.")]);
    let handle = model.clone();
    let memory = harness.memory(model).await;
    let (mut typing, mut input) = typed_input();
    let (sender, mut notices) = mpsc::unbounded_channel();
    let mut presenter = Notices(sender);

    let read = read_line_during_idle_work(&mut input, Some(&memory), &mut presenter);
    let driver = async {
        // The notice arrives before anything is typed: it is shown while idle.
        assert_eq!(
            notices.recv().await.as_deref(),
            Some("Noted: The user's cat is called Miso.")
        );
        typing.write_all(b"hello\n").await.expect("typing works");
    };
    let (result, ()) = tokio::join!(read, driver);

    let (line, warning) = result.expect("the read succeeds");
    assert_line(&line, "hello");
    assert_eq!(
        warning, None,
        "an embedding server that is down is not news"
    );
    assert_eq!(
        harness.fact_texts().await,
        ["The user's cat is called Miso."]
    );
    assert!(!harness.turn_is_waiting().await);
    // Only Darren's words were sent, and the turn was asked about once.
    assert_eq!(handle.request_count(), 1);
    assert!(!format!("{:?}", handle.requests()[0]).contains("ANSWER-JARVIS-GAVE"));
}

#[tokio::test]
async fn a_turn_with_nothing_to_note_shows_nothing() {
    let harness = Harness::open("What time is it?").await;
    let memory = harness
        .memory(MockCompletionModel::from_stream_turns([answer("NONE")]))
        .await;
    let (mut typing, mut input) = typed_input();
    let (sender, mut notices) = mpsc::unbounded_channel();
    let mut presenter = Notices(sender);

    let read = read_line_during_idle_work(&mut input, Some(&memory), &mut presenter);
    let driver = async {
        while harness.turn_is_waiting().await {
            tokio::task::yield_now().await;
        }
        typing.write_all(b"hello\n").await.expect("typing works");
    };
    let (result, ()) = tokio::join!(read, driver);

    let (line, warning) = result.expect("the read succeeds");
    assert_line(&line, "hello");
    assert_eq!(warning, None);
    drop(presenter);
    assert_eq!(notices.recv().await, None, "nothing was shown");
    assert!(harness.fact_texts().await.is_empty());
}

#[tokio::test]
async fn a_line_typed_during_the_model_run_is_handled_at_once_and_the_turn_is_examined_later() {
    let harness = Harness::open("My cat is called Miso.").await;
    let model = SignallingModel::default();
    let started = Arc::clone(&model.started);
    let memory = harness.memory(model).await;
    let (mut typing, mut input) = typed_input();
    let (sender, mut notices) = mpsc::unbounded_channel();
    let mut presenter = Notices(sender);

    {
        let read = read_line_during_idle_work(&mut input, Some(&memory), &mut presenter);
        tokio::pin!(read);
        // The model has the request and is still working when the line is typed.
        tokio::select! {
            _ = &mut read => panic!("the model never answers, so the read is still idle"),
            () = started.notified() => {}
        }
        typing.write_all(b"hello\n").await.expect("typing works");

        let (line, warning) = read.await.expect("the read succeeds");
        assert_line(&line, "hello");
        assert_eq!(warning, None, "a cut-off pass is not a failure");
    }
    assert!(harness.turn_is_waiting().await);
    assert!(harness.fact_texts().await.is_empty());

    // The next idle examines it.
    let later = harness
        .memory(MockCompletionModel::from_stream_turns([answer(
            "FACT: The user's cat is called Miso.",
        )]))
        .await;
    let read = read_line_during_idle_work(&mut input, Some(&later), &mut presenter);
    let driver = async {
        assert_eq!(
            notices.recv().await.as_deref(),
            Some("Noted: The user's cat is called Miso.")
        );
        typing.write_all(b"next\n").await.expect("typing works");
    };
    let (result, ()) = tokio::join!(read, driver);
    let (line, _) = result.expect("the read succeeds");
    assert_line(&line, "next");
    assert!(!harness.turn_is_waiting().await);
    assert_eq!(harness.fact_texts().await.len(), 1);
}

#[tokio::test]
async fn dropping_the_whole_idle_read_during_the_model_run_leaves_the_turn_unexamined() {
    // A tap drops the idle read as a whole.
    let harness = Harness::open("My cat is called Miso.").await;
    let model = SignallingModel::default();
    let started = Arc::clone(&model.started);
    let memory = harness.memory(model).await;
    let (_typing, mut input) = typed_input();
    let (sender, _notices) = mpsc::unbounded_channel();
    let mut presenter = Notices(sender);

    tokio::select! {
        _ = read_line_during_idle_work(&mut input, Some(&memory), &mut presenter) => {
            panic!("the model never answers, so the read is still idle")
        }
        () = started.notified() => {}
    }

    assert!(harness.turn_is_waiting().await);
    assert!(harness.fact_texts().await.is_empty());
}

#[tokio::test]
async fn a_failing_model_costs_one_warning_and_one_request_not_a_loop() {
    let harness = Harness::open("My cat is called Miso.").await;
    let model = MockCompletionModel::from_stream_turns([vec![
        MockStreamEvent::text("FACT: partial"),
        MockStreamEvent::error("injected failure with SECRET-MODEL-TEXT"),
    ]]);
    let handle = model.clone();
    let memory = harness.memory(model).await;
    let (mut typing, mut input) = typed_input();
    let (sender, _notices) = mpsc::unbounded_channel();
    let mut presenter = Notices(sender);

    let read = read_line_during_idle_work(&mut input, Some(&memory), &mut presenter);
    let driver = async {
        while handle.request_count() == 0 {
            tokio::task::yield_now().await;
        }
        // Long enough for a loop that retried at once to have asked again.
        let_others_run().await;
        typing.write_all(b"hello\n").await.expect("typing works");
    };
    let (result, ()) = tokio::join!(read, driver);

    let (line, warning) = result.expect("the read succeeds");
    assert_line(&line, "hello");
    let warning = warning.expect("a failed pass says so");
    assert_eq!(warning, "fact noting paused: the model run failed");
    assert!(!warning.contains("SECRET"), "no model text in the warning");
    assert_eq!(handle.request_count(), 1);
    assert!(
        harness.turn_is_waiting().await,
        "a failure leaves the turn unexamined"
    );
}

#[tokio::test]
async fn without_a_noting_pass_the_idle_time_is_just_waiting_for_input() {
    let harness = Harness::open("My cat is called Miso.").await;
    let memory = MemoryService::open(
        harness.archive.memory_repository(),
        RemoteEmbeddings::new(&harness.embeddings).expect("a localhost URL is allowed"),
    );
    memory.wait_until_embedder_idle().await;
    let (mut typing, mut input) = typed_input();
    let (sender, _notices) = mpsc::unbounded_channel();
    let mut presenter = Notices(sender);
    typing.write_all(b"hello\n").await.expect("typing works");

    let (line, warning) = read_line_during_idle_work(&mut input, Some(&memory), &mut presenter)
        .await
        .expect("the read succeeds");

    assert_line(&line, "hello");
    assert_eq!(warning, None);
    assert!(harness.turn_is_waiting().await);
}

#[test]
fn noted_facts_are_shown_one_to_a_line_and_dropped_ones_in_a_single_line() {
    fn fact(id: i64, text: &str) -> Fact {
        Fact {
            id: FactId(id),
            text: text.to_owned(),
            origin: FactOrigin::Noted,
            created_at_unix_ms: id,
        }
    }
    let cases = [
        ("none kept", Examination::default(), Vec::<&str>::new()),
        (
            "two kept",
            Examination {
                noted: vec![
                    fact(3, "The user likes tea."),
                    fact(4, "The user lives in Leeds."),
                ],
                dropped: Vec::new(),
            },
            vec![
                "Noted: The user likes tea.",
                "Noted: The user lives in Leeds.",
            ],
        ),
        (
            "one kept, two dropped",
            Examination {
                noted: vec![fact(5, "The user likes tea.")],
                dropped: vec![fact(1, "older one"), fact(2, "older two")],
            },
            vec![
                "Noted: The user likes tea.",
                "Dropped to make room for them: older one; older two",
            ],
        ),
    ];
    for (name, examination, expected) in cases {
        let (sender, mut notices) = mpsc::unbounded_channel();
        let mut presenter = Notices(sender);
        show_noted(&mut presenter, &examination).expect("notices are shown");
        drop(presenter);
        let mut shown = Vec::new();
        while let Ok(line) = notices.try_recv() {
            shown.push(line);
        }
        assert_eq!(shown, expected, "{name}");
    }
}
