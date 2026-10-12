use std::path::PathBuf;

use crate::features::{
    control::{MemoryCommand, Notice},
    conversation::ConversationSession,
    memory::MemoryService,
};
use crate::storage::{ArchiveOutcome, ConversationArchive, MemoryEligibility, SourceId};

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
}

impl Fixture {
    async fn open(label: &str) -> Self {
        let path = database_path(label);
        let mut session = ConversationSession::new(path.clone());
        session.reset().await.expect("session archive opens");
        let archive = ConversationArchive::open(&path)
            .await
            .expect("archive opens");
        let memory = MemoryService::open(
            session
                .memory_repository()
                .expect("repository is available"),
            PathBuf::from("/nonexistent/jarvis-embeddings"),
        );
        Self {
            path,
            session,
            archive,
            memory,
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
