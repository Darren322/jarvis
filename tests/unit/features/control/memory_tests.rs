use std::path::PathBuf;

use wiremock::MockServer;

use crate::clients::remote_embeddings::RemoteEmbeddings;
use crate::config::EmbeddingConfig;
use crate::features::{
    control::{MemoryCommand, Notice},
    conversation::ConversationSession,
    memory::MemoryService,
};
use crate::storage::{
    ArchiveOutcome, ConversationArchive, FactOrigin, MAX_FACT_BYTES, MemoryEligibility, SourceId,
};

use super::{Focus, MemoryControl};

fn database_path(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "jarvis-control-{label}-{}-{}.sqlite3",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock is after Unix epoch")
            .as_nanos()
    ))
}

struct Fixture {
    path: PathBuf,
    session: ConversationSession,
    archive: ConversationArchive,
    memory: MemoryService,
    /// No routes, so the embedder is unavailable and recall is keyword-only.
    _embedding_server: MockServer,
}

impl Fixture {
    async fn open(label: &str) -> Self {
        let path = database_path(label);
        let mut session = ConversationSession::new(path.clone());
        session.reset().await.expect("session archive opens");
        let archive = ConversationArchive::open(&path)
            .await
            .expect("archive opens");
        let embedding_server = MockServer::start().await;
        let embeddings = RemoteEmbeddings::new(&EmbeddingConfig {
            base_url: format!("{}/v1", embedding_server.uri()),
            model: "bge-test".to_owned(),
        })
        .expect("a localhost URL is allowed");
        let memory = MemoryService::open(
            session
                .memory_repository()
                .expect("repository is available"),
            embeddings,
        );
        Self {
            path,
            session,
            archive,
            memory,
            _embedding_server: embedding_server,
        }
    }

    async fn remember(&self, user: &str) -> SourceId {
        self.archive
            .append_turn(
                user,
                ArchiveOutcome::Completed("Noted.".to_owned()),
                MemoryEligibility::Eligible,
                &[],
            )
            .await
            .expect("turn is archived")
            .source_id
            .expect("eligible turn has a source")
    }

    async fn run(&mut self, control: &mut MemoryControl, command: MemoryCommand) -> Vec<Notice> {
        control
            .run(command, Some(&self.memory), &mut self.session)
            .await
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn says(notices: &[Notice], text: &str) -> bool {
    notices.iter().any(|notice| match notice {
        Notice::Info(line) | Notice::Warning(line) => line.contains(text),
    })
}

#[tokio::test]
async fn a_listing_focuses_one_result_but_not_zero_or_two() {
    let mut fixture = Fixture::open("listing").await;
    fixture.remember("My cat is Miso.").await;
    fixture.remember("My dog is Rex.").await;
    let mut control = MemoryControl::default();

    fixture
        .run(&mut control, MemoryCommand::List(Some("nothing".into())))
        .await;
    assert_eq!(control.focus, None);

    fixture
        .run(&mut control, MemoryCommand::List(Some("Miso".into())))
        .await;
    assert!(matches!(control.focus, Some(Focus::Memory(_))));

    fixture.run(&mut control, MemoryCommand::List(None)).await;
    assert_eq!(control.focus, Some(Focus::AmbiguousDisplay));
    let notices = fixture.run(&mut control, MemoryCommand::ForgetFocus).await;
    assert!(says(&notices, "nothing was forgotten"));
}

#[tokio::test]
async fn forget_that_targets_the_last_turn_then_clears_the_focus() {
    let mut fixture = Fixture::open("forget-focus").await;
    let mut control = MemoryControl::default();
    let notices = fixture.run(&mut control, MemoryCommand::ForgetFocus).await;
    assert!(says(&notices, "no current memory"));

    let cat = fixture.remember("My cat is Miso.").await;
    control.focus_on_turn(Some(cat));
    let notices = fixture.run(&mut control, MemoryCommand::ForgetFocus).await;
    assert!(says(&notices, "Forgot 1 memories from 1 sources"));
    assert_eq!(control.focus, None);

    control.focus_on_turn(Some(cat));
    control.clear();
    assert_eq!(control.focus, None);
}

#[tokio::test]
async fn forget_detail_needs_exactly_one_match() {
    let mut fixture = Fixture::open("forget-detail").await;
    fixture.remember("My cat is Miso.").await;
    fixture.remember("My cat's vet is Dr Lee.").await;
    let mut control = MemoryControl::default();

    let none = fixture
        .run(&mut control, MemoryCommand::ForgetDetail("parrot".into()))
        .await;
    assert!(says(&none, "could not find one memory"));
    let two = fixture
        .run(&mut control, MemoryCommand::ForgetDetail("cat".into()))
        .await;
    assert!(says(&two, "matches multiple memories"));
    let one = fixture
        .run(&mut control, MemoryCommand::ForgetDetail("Miso".into()))
        .await;
    assert!(says(&one, "Forgot 1 memories"));
}

#[tokio::test]
async fn every_memory_command_reports_unavailable_storage_instead_of_failing() {
    let path = database_path("unavailable");
    let mut session = ConversationSession::new(path.clone());
    let mut control = MemoryControl::default();
    let notices = control.run(MemoryCommand::Status, None, &mut session).await;
    assert_eq!(
        notices,
        [Notice::Info("Local memory storage is unavailable.".into())]
    );
    let _ = std::fs::remove_file(&path);
}

async fn fact_texts(session: &ConversationSession) -> Vec<String> {
    session
        .facts_repository()
        .expect("the archive is open")
        .list()
        .await
        .expect("facts list")
        .into_iter()
        .map(|fact| fact.text)
        .collect()
}

fn info(text: &str) -> Notice {
    Notice::Info(text.to_owned())
}

#[tokio::test]
async fn remembering_saves_a_stated_fact_says_got_it_and_focuses_it() {
    let fixture = Fixture::open("remember").await;
    let mut control = MemoryControl::default();

    let notices = control
        .remember("  my cat is Miso ", &fixture.session)
        .await;

    assert_eq!(notices, [info("Got it.")]);
    let facts = fixture
        .session
        .facts_repository()
        .expect("the archive is open")
        .list()
        .await
        .expect("facts list");
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0].text, "my cat is Miso");
    assert_eq!(facts[0].origin, FactOrigin::Stated);
    assert_eq!(control.focus, Some(Focus::Fact(facts[0].id)));
}

#[tokio::test]
async fn forget_that_right_after_a_save_removes_that_fact_only() {
    let mut fixture = Fixture::open("forget-fact").await;
    let mut control = MemoryControl::default();
    control.remember("my dog is Rex", &fixture.session).await;
    control.remember("my cat is Miso", &fixture.session).await;

    let notices = fixture.run(&mut control, MemoryCommand::ForgetFocus).await;

    assert_eq!(notices, [info("Forgot that: my cat is Miso")]);
    assert_eq!(fact_texts(&fixture.session).await, ["my dog is Rex"]);
    assert_eq!(control.focus, None);
    let again = fixture.run(&mut control, MemoryCommand::ForgetFocus).await;
    assert!(says(&again, "no current memory"));
    assert_eq!(fact_texts(&fixture.session).await, ["my dog is Rex"]);
}

#[tokio::test]
async fn forget_that_removes_a_saved_fact_even_without_a_memory_service() {
    let mut fixture = Fixture::open("forget-fact-no-memory").await;
    let mut control = MemoryControl::default();
    control.remember("my cat is Miso", &fixture.session).await;

    let notices = control
        .run(MemoryCommand::ForgetFocus, None, &mut fixture.session)
        .await;

    assert_eq!(notices, [info("Forgot that: my cat is Miso")]);
    assert!(fact_texts(&fixture.session).await.is_empty());
}

#[tokio::test]
async fn another_command_moves_the_focus_off_the_saved_fact() {
    let mut fixture = Fixture::open("focus-moves").await;
    let mut control = MemoryControl::default();
    control.remember("my cat is Miso", &fixture.session).await;

    fixture
        .run(&mut control, MemoryCommand::List(Some("nothing".into())))
        .await;
    let notices = fixture.run(&mut control, MemoryCommand::ForgetFocus).await;

    assert!(says(&notices, "no current memory"));
    assert_eq!(fact_texts(&fixture.session).await, ["my cat is Miso"]);

    // A successful prompt (or /reset) moves it too.
    control.remember("my dog is Rex", &fixture.session).await;
    control.focus_on_turn(None);
    assert_eq!(control.focus, None);
    control.remember("my fish is Bob", &fixture.session).await;
    control.clear();
    assert_eq!(control.focus, None);
}

#[tokio::test]
async fn forgetting_a_fact_that_is_already_gone_says_so() {
    let mut fixture = Fixture::open("forget-gone").await;
    let mut control = MemoryControl::default();
    control.remember("my cat is Miso", &fixture.session).await;
    let facts = fixture.session.facts_repository().expect("archive is open");
    let saved = facts.list().await.expect("facts list");
    facts.remove(saved[0].id).await.expect("fact removed");

    let notices = fixture.run(&mut control, MemoryCommand::ForgetFocus).await;

    assert_eq!(
        notices,
        [info("That fact is already gone; nothing was forgotten.")]
    );
}

#[tokio::test]
async fn each_refused_save_says_why_in_plain_words_and_clears_the_focus() {
    let fixture = Fixture::open("refusals").await;
    let mut control = MemoryControl::default();
    let facts = fixture.session.facts_repository().expect("archive is open");

    control.remember("my cat is Miso", &fixture.session).await;
    assert!(control.focus.is_some());
    let too_long = "x".repeat(MAX_FACT_BYTES + 1);
    let notices = control.remember(&too_long, &fixture.session).await;
    assert_eq!(
        notices,
        [info(
            "That is too long for me to remember. Keep it to 200 characters or fewer."
        )]
    );
    assert_eq!(control.focus, None);

    // The last accepted size is saved; the first refused one was refused above.
    let longest = "y".repeat(MAX_FACT_BYTES);
    assert_eq!(
        control.remember(&longest, &fixture.session).await,
        [info("Got it.")]
    );

    let notices = control.remember("two\nlines", &fixture.session).await;
    assert_eq!(notices, [info("I can only remember one line at a time.")]);

    let notices = control.remember("   ", &fixture.session).await;
    assert_eq!(notices.len(), 1);
    assert!(says(&notices, "Use /remember"));

    // Stated facts fill the list; the next save is refused and nothing is dropped.
    let mut saved = 0;
    let full = loop {
        let notices = control.remember(&longest, &fixture.session).await;
        if notices != [info("Got it.")] {
            break notices;
        }
        saved += 1;
        assert!(saved < 10, "the list must fill up");
    };
    assert_eq!(
        full,
        [info(
            "I can't remember more: the list is full of things you told me. Forget one first."
        )]
    );
    assert_eq!(control.focus, None);
    assert_eq!(
        facts.list().await.expect("facts list").len(),
        2 + saved,
        "a refused save changes nothing"
    );
}

#[tokio::test]
async fn a_save_that_drops_noted_facts_says_which() {
    let fixture = Fixture::open("dropped").await;
    let facts = fixture.session.facts_repository().expect("archive is open");
    let oldest = format!("a{}", "n".repeat(MAX_FACT_BYTES - 1));
    facts
        .insert(&oldest, FactOrigin::Noted)
        .await
        .expect("noted fact saved");
    for _ in 0..4 {
        facts
            .insert(&"n".repeat(MAX_FACT_BYTES), FactOrigin::Noted)
            .await
            .expect("noted fact saved");
    }
    let mut control = MemoryControl::default();

    let notices = control
        .remember(&"s".repeat(MAX_FACT_BYTES), &fixture.session)
        .await;

    assert_eq!(
        notices,
        [
            info("Got it."),
            info(&format!("To make room I forgot: {oldest}."))
        ]
    );
}

#[tokio::test]
async fn a_save_with_no_storage_says_so() {
    let path = database_path("remember-unavailable");
    let session = ConversationSession::new(path.clone());
    let mut control = MemoryControl::default();

    let notices = control.remember("my cat is Miso", &session).await;

    assert_eq!(
        notices,
        [info(
            "I can't save that: local memory storage is unavailable."
        )]
    );
    assert_eq!(control.focus, None);
    let _ = std::fs::remove_file(&path);
}
