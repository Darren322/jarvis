use std::sync::Arc;

use rig_agent::prelude::{CompletionError, CompletionModel};
use rig_core::{
    completion::{CompletionRequest, CompletionResponse, Usage},
    streaming::StreamingCompletionResponse,
    test_utils::{MockCompletionModel, MockStreamEvent},
};

use tempfile::TempDir;
use tokio::sync::Notify;

use crate::features::assistant::FactExtractor;
use crate::storage::{ArchiveOutcome, ConversationArchive, FactsRepository, MemoryEligibility};

use super::{FactNoter, MAX_ATTEMPTS_PER_TURN, MAX_FACTS_PER_TURN, NotingError, parse_candidates};

/// A temporary database that is removed when the test ends.
struct Fixture {
    _directory: TempDir,
    archive: ConversationArchive,
}

impl Fixture {
    async fn open(_label: &str) -> Self {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let archive = ConversationArchive::open(directory.path().join("jarvis.sqlite3"))
            .await
            .expect("archive opens");
        Self {
            _directory: directory,
            archive,
        }
    }

    fn facts(&self) -> FactsRepository {
        self.archive.facts_repository()
    }

    fn noter(&self, model: impl CompletionModel + 'static) -> FactNoter {
        FactNoter::new(FactExtractor::new(model), self.facts())
    }

    async fn say(&self, said: &str, answer: &str) {
        self.archive
            .append_turn(
                said,
                ArchiveOutcome::Completed(answer.to_owned()),
                MemoryEligibility::Eligible,
                &[],
            )
            .await
            .expect("turn is archived");
    }

    async fn fact_texts(&self) -> Vec<String> {
        self.facts()
            .list()
            .await
            .expect("facts list")
            .into_iter()
            .map(|fact| fact.text)
            .collect()
    }

    async fn turn_is_waiting(&self) -> bool {
        self.facts()
            .next_unexamined_turn(&[])
            .await
            .expect("next turn is read")
            .is_some()
    }
}

fn answer(text: &str) -> Vec<MockStreamEvent> {
    vec![
        MockStreamEvent::text(text),
        MockStreamEvent::final_response(Usage::new()),
    ]
}

fn failure() -> Vec<MockStreamEvent> {
    vec![
        MockStreamEvent::text("FACT: partial"),
        MockStreamEvent::error("injected failure"),
    ]
}

/// Examines the next turn through both phases; `None` when no turn was waiting.
async fn examine(noter: &FactNoter) -> Option<Vec<String>> {
    let extracted = noter
        .extract_next_turn()
        .await
        .expect("the model run succeeds")?;
    let examination = noter.commit(extracted).await.expect("the commit succeeds");
    Some(
        examination
            .noted
            .into_iter()
            .map(|fact| fact.text)
            .collect(),
    )
}

#[test]
fn only_lines_that_start_with_the_marker_are_facts() {
    let lines =
        |count: usize| -> String { (1..=count).map(|n| format!("FACT: fact {n}\n")).collect() };
    let cases: [(&str, &str, Vec<&str>); 11] = [
        ("no facts", "NONE", vec![]),
        ("nothing at all", "", vec![]),
        (
            "one fact",
            "FACT: The user likes tea.",
            vec!["The user likes tea."],
        ),
        (
            "several, with spacing and a Windows line end",
            "FACT: a\r\n  FACT:   b  \nFACT: c",
            vec!["a", "b", "c"],
        ),
        ("an empty fact", "FACT:\nFACT:   \n", vec![]),
        ("prose around nothing", "Sure! The user likes tea.", vec![]),
        (
            "a bullet before the marker",
            "- FACT: The user likes tea.",
            vec![],
        ),
        ("a lower-case marker", "fact: The user likes tea.", vec![]),
        (
            "a marker inside a line",
            "Note FACT: The user likes tea.",
            vec![],
        ),
        (
            "NONE beside a fact keeps the fact",
            "NONE\nFACT: The user likes tea.",
            vec!["The user likes tea."],
        ),
        (
            "the marker, not a fact word",
            "FACTS: a\nFACTUAL: b",
            vec![],
        ),
    ];
    for (name, output, expected) in cases {
        assert_eq!(parse_candidates(output), expected, "{name}");
    }
    // Five facts in one answer are all kept; a sixth is dropped.
    assert_eq!(MAX_FACTS_PER_TURN, 5);
    assert_eq!(
        parse_candidates(&lines(5)),
        ["fact 1", "fact 2", "fact 3", "fact 4", "fact 5"]
    );
    assert_eq!(parse_candidates(&lines(6)), parse_candidates(&lines(5)));
}

#[tokio::test]
async fn a_turn_stating_a_fact_yields_a_noted_fact() {
    let fixture = Fixture::open("fact").await;
    fixture
        .say("My cat is called Miso.", "Lovely, master.")
        .await;
    let noter = fixture.noter(MockCompletionModel::from_stream_turns([answer(
        "FACT: The user's cat is called Miso.",
    )]));

    let noted = examine(&noter).await.expect("a turn was waiting");

    assert_eq!(noted, ["The user's cat is called Miso."]);
    assert_eq!(
        fixture.fact_texts().await,
        ["The user's cat is called Miso."]
    );
    assert!(!fixture.turn_is_waiting().await);
}

#[tokio::test]
async fn a_turn_with_no_fact_yields_nothing_and_is_not_asked_about_again() {
    let fixture = Fixture::open("none").await;
    fixture.say("What time is it?", "Nearly tea time.").await;
    let model = MockCompletionModel::from_stream_turns([answer("NONE")]);
    let handle = model.clone();
    let noter = fixture.noter(model);

    assert_eq!(examine(&noter).await, Some(Vec::new()));
    assert_eq!(examine(&noter).await, None);

    assert!(fixture.fact_texts().await.is_empty());
    assert_eq!(
        handle.request_count(),
        1,
        "an examined turn is not re-asked"
    );
}

#[tokio::test]
async fn malformed_output_yields_nothing_and_no_error() {
    let outputs = [
        "Sure! Here are the facts: the user likes tea.",
        "{\"facts\": [\"The user likes tea.\"]}",
        "FACT:",
        "```\nFACT\n```",
    ];
    for output in outputs {
        let fixture = Fixture::open("malformed").await;
        fixture.say("I like tea.", "Noted.").await;
        let noter = fixture.noter(MockCompletionModel::from_stream_turns([answer(output)]));

        assert_eq!(examine(&noter).await, Some(Vec::new()), "{output}");

        assert!(fixture.fact_texts().await.is_empty(), "{output}");
        assert!(!fixture.turn_is_waiting().await, "{output}");
    }
}

#[tokio::test]
async fn output_is_held_to_the_fact_limits_and_duplicates_are_skipped() {
    let fixture = Fixture::open("limits").await;
    fixture
        .facts()
        .insert(
            "The user's sister is called Mia.",
            crate::storage::FactOrigin::Stated,
        )
        .await
        .expect("a stated fact is added");
    fixture.say("lots of talk", "ok").await;
    let too_long = "x".repeat(201);
    let model_output = format!(
        "FACT: the user's sister is called mia.\nFACT: {too_long}\nFACT: The user lives in Leeds.\n"
    );
    let noter = fixture.noter(MockCompletionModel::from_stream_turns([answer(
        &model_output,
    )]));

    let noted = examine(&noter).await.expect("a turn was waiting");

    assert_eq!(noted, ["The user lives in Leeds."]);
    assert_eq!(
        fixture.fact_texts().await,
        [
            "The user's sister is called Mia.",
            "The user lives in Leeds."
        ]
    );
}

#[tokio::test]
async fn the_request_holds_what_the_user_said_and_none_of_the_answer() {
    let fixture = Fixture::open("request").await;
    fixture
        .say(
            "My cat is called Miso.",
            "ANSWER-ONLY-JARVIS-SAID-THIS, master.",
        )
        .await;
    fixture.say("I live in Leeds.", "MORE-JARVIS-WORDS.").await;
    let model = MockCompletionModel::from_stream_turns([answer("NONE"), answer("NONE")]);
    let handle = model.clone();
    let noter = fixture.noter(model);

    examine(&noter).await.expect("the first turn");
    examine(&noter).await.expect("the second turn");

    let requests = handle.requests();
    assert_eq!(requests.len(), 2);
    for (request, said) in requests
        .iter()
        .zip(["My cat is called Miso.", "I live in Leeds."])
    {
        let sent = format!("{request:?}");
        assert!(sent.contains(said), "{said}");
        assert!(!sent.contains("JARVIS"), "no answer text in the request");
        assert!(request.tools.is_empty());
    }
}

#[tokio::test]
async fn dropped_failed_and_examined_turns_are_never_sent() {
    let fixture = Fixture::open("skipped").await;
    for outcome in [
        ArchiveOutcome::TimedOut,
        ArchiveOutcome::PromptFailed,
        ArchiveOutcome::InvalidResponse,
    ] {
        fixture
            .archive
            .append_turn(
                "A DROPPED TURN",
                outcome,
                MemoryEligibility::ArchiveOnly,
                &[],
            )
            .await
            .expect("turn is archived");
    }
    fixture.say("My cat is called Miso.", "Lovely.").await;
    let model = MockCompletionModel::from_stream_turns([answer("NONE"), answer("NONE")]);
    let handle = model.clone();
    let noter = fixture.noter(model);

    assert!(examine(&noter).await.is_some());
    assert_eq!(examine(&noter).await, None);

    let requests = handle.requests();
    assert_eq!(requests.len(), 1);
    assert!(!format!("{:?}", requests[0]).contains("A DROPPED TURN"));
}

#[tokio::test]
async fn a_failed_model_run_leaves_the_turn_unexamined_and_it_is_asked_about_again() {
    let fixture = Fixture::open("failure").await;
    fixture.say("My cat is called Miso.", "Lovely.").await;
    let noter = fixture.noter(MockCompletionModel::from_stream_turns([
        failure(),
        answer("FACT: The user's cat is called Miso."),
    ]));

    let error = noter
        .extract_next_turn()
        .await
        .expect_err("the first run fails");
    assert!(matches!(error, NotingError::Model(_)), "{error:?}");
    assert!(fixture.turn_is_waiting().await);
    assert!(fixture.fact_texts().await.is_empty());

    assert_eq!(
        examine(&noter).await,
        Some(vec!["The user's cat is called Miso.".to_owned()])
    );
}

#[tokio::test]
async fn a_turn_that_keeps_failing_is_given_up_after_the_maximum_attempts() {
    let fixture = Fixture::open("give-up").await;
    fixture.say("FIRST-TURN always fails", "ok").await;
    fixture.say("SECOND-TURN is fine", "ok").await;
    let failures = (0..MAX_ATTEMPTS_PER_TURN).map(|_| failure());
    let model = MockCompletionModel::from_stream_turns(
        failures
            .chain([answer("FACT: The user is fine.")])
            .collect::<Vec<_>>(),
    );
    let handle = model.clone();
    let noter = fixture.noter(model);

    // Every failure up to the maximum is the first turn again...
    for _ in 0..MAX_ATTEMPTS_PER_TURN {
        assert!(noter.extract_next_turn().await.is_err());
    }
    // ...and then the pass moves to the next turn instead of retrying for ever.
    assert_eq!(
        examine(&noter).await,
        Some(vec!["The user is fine.".to_owned()])
    );

    let sent: Vec<bool> = handle
        .requests()
        .iter()
        .map(|request| format!("{request:?}").contains("FIRST-TURN"))
        .collect();
    let mut expected = vec![true; usize::from(MAX_ATTEMPTS_PER_TURN)];
    expected.push(false);
    assert_eq!(sent, expected);
    // The first turn was never marked examined: a restart tries it again.
    assert!(fixture.turn_is_waiting().await);
}

#[tokio::test]
async fn one_failure_fewer_than_the_maximum_still_retries_the_turn() {
    let fixture = Fixture::open("retry").await;
    fixture.say("FIRST-TURN fails twice", "ok").await;
    fixture.say("SECOND-TURN", "ok").await;
    let failures = (0..MAX_ATTEMPTS_PER_TURN - 1).map(|_| failure());
    let model = MockCompletionModel::from_stream_turns(
        failures.chain([answer("NONE")]).collect::<Vec<_>>(),
    );
    let handle = model.clone();
    let noter = fixture.noter(model);

    for _ in 0..MAX_ATTEMPTS_PER_TURN - 1 {
        assert!(noter.extract_next_turn().await.is_err());
    }
    examine(&noter)
        .await
        .expect("the first turn is tried again");

    let last = format!(
        "{:?}",
        handle.requests().last().expect("a request was sent")
    );
    assert!(last.contains("FIRST-TURN"));
}

#[derive(Clone)]
struct PendingModel;

impl CompletionModel for PendingModel {
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
        std::future::pending().await
    }
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

#[tokio::test(start_paused = true)]
async fn a_model_that_never_answers_is_a_timeout_and_leaves_the_turn_unexamined() {
    let fixture = Fixture::open("timeout").await;
    fixture.say("My cat is called Miso.", "Lovely.").await;
    let noter = fixture.noter(PendingModel);

    let error = noter
        .extract_next_turn()
        .await
        .expect_err("the run times out");

    assert!(matches!(error, NotingError::TimedOut), "{error:?}");
    assert_eq!(error.to_string(), "the model did not answer in time");
    assert!(fixture.turn_is_waiting().await);
}

#[tokio::test]
async fn a_run_dropped_mid_way_leaves_the_turn_unexamined_and_adds_no_fact() {
    let fixture = Fixture::open("dropped").await;
    fixture.say("My cat is called Miso.", "Lovely.").await;
    let model = SignallingModel::default();
    let started = Arc::clone(&model.started);
    let hanging = fixture.noter(model);

    // The run is abandoned once the model has the request and has not answered, as a
    // typed line abandons it. Dropping happens when `select!` returns.
    tokio::select! {
        _ = hanging.extract_next_turn() => panic!("the model never answers"),
        () = started.notified() => {}
    }

    assert!(fixture.turn_is_waiting().await);
    assert!(fixture.fact_texts().await.is_empty());
    // A later pass examines it, once.
    let later = fixture.noter(MockCompletionModel::from_stream_turns([answer(
        "FACT: The user's cat is called Miso.",
    )]));
    assert_eq!(
        examine(&later).await,
        Some(vec!["The user's cat is called Miso.".to_owned()])
    );
    assert_eq!(examine(&later).await, None);
    assert_eq!(fixture.fact_texts().await.len(), 1);
}
