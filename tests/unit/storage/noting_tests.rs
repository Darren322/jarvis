use tempfile::TempDir;

use crate::storage::{
    ArchiveOutcome, ConversationArchive, FactOrigin, FactsRepository, MemoryEligibility, TurnId,
};

struct Fixture {
    _directory: TempDir,
    archive: ConversationArchive,
    facts: FactsRepository,
}

impl Fixture {
    async fn open(_label: &str) -> Self {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let archive = ConversationArchive::open(directory.path().join("jarvis.sqlite3"))
            .await
            .expect("archive opens");
        let facts = archive.facts_repository();
        Self {
            _directory: directory,
            archive,
            facts,
        }
    }

    /// Archives a completed turn and returns its id.
    async fn completed(&self, said: &str) -> TurnId {
        let receipt = self
            .archive
            .append_turn(
                said,
                ArchiveOutcome::Completed("An answer that must never be read.".to_owned()),
                MemoryEligibility::Eligible,
                &[],
            )
            .await
            .expect("turn is archived");
        TurnId(receipt.turn_id)
    }

    async fn failed(&self, said: &str, outcome: ArchiveOutcome) {
        self.archive
            .append_turn(said, outcome, MemoryEligibility::ArchiveOnly, &[])
            .await
            .expect("turn is archived");
    }

    async fn next_text(&self, skipped: &[TurnId]) -> Option<String> {
        self.facts
            .next_unexamined_turn(skipped)
            .await
            .expect("the next turn is read")
            .map(|turn| turn.user_text)
    }

    async fn texts(&self) -> Vec<String> {
        self.facts
            .list()
            .await
            .expect("facts list")
            .into_iter()
            .map(|fact| fact.text)
            .collect()
    }
}

fn strings(texts: &[&str]) -> Vec<String> {
    texts.iter().map(|text| (*text).to_owned()).collect()
}

#[tokio::test]
async fn turns_are_examined_oldest_first_and_only_completed_ones() {
    let fixture = Fixture::open("order").await;
    fixture.completed("first").await;
    fixture.failed("timed out", ArchiveOutcome::TimedOut).await;
    fixture.failed("failed", ArchiveOutcome::PromptFailed).await;
    fixture
        .failed("invalid", ArchiveOutcome::InvalidResponse)
        .await;
    let second = fixture.completed("second").await;

    let first = fixture
        .facts
        .next_unexamined_turn(&[])
        .await
        .expect("read")
        .expect("a turn waits");
    assert_eq!(first.user_text, "first");
    fixture
        .facts
        .finish_examination(first.id, Vec::new())
        .await
        .expect("examined");

    let next = fixture
        .facts
        .next_unexamined_turn(&[])
        .await
        .expect("read")
        .expect("a turn waits");
    assert_eq!((next.id, next.user_text.as_str()), (second, "second"));
    fixture
        .facts
        .finish_examination(next.id, Vec::new())
        .await
        .expect("examined");
    assert_eq!(fixture.next_text(&[]).await, None);
}

#[tokio::test]
async fn a_skipped_turn_is_passed_over_without_being_examined() {
    let fixture = Fixture::open("skipped").await;
    let first = fixture.completed("first").await;
    fixture.completed("second").await;

    assert_eq!(fixture.next_text(&[first]).await.as_deref(), Some("second"));
    assert_eq!(fixture.next_text(&[]).await.as_deref(), Some("first"));
}

#[tokio::test]
async fn a_turn_whose_memory_was_forgotten_is_never_examined() {
    let fixture = Fixture::open("forgotten").await;
    let receipt = fixture
        .archive
        .append_turn(
            "My PIN is 4321.",
            ArchiveOutcome::Completed("Noted.".to_owned()),
            MemoryEligibility::Eligible,
            &[],
        )
        .await
        .expect("turn is archived");
    fixture
        .archive
        .memory_repository()
        .forget_source(receipt.source_id.expect("an eligible turn has a source"))
        .await
        .expect("source is forgotten");
    fixture.completed("My cat is called Miso.").await;

    assert_eq!(
        fixture.next_text(&[]).await.as_deref(),
        Some("My cat is called Miso.")
    );
}

#[tokio::test]
async fn forgetting_a_turn_while_it_is_being_examined_adds_nothing() {
    let fixture = Fixture::open("forgotten-midway").await;
    let receipt = fixture
        .archive
        .append_turn(
            "My PIN is 4321.",
            ArchiveOutcome::Completed("Noted.".to_owned()),
            MemoryEligibility::Eligible,
            &[],
        )
        .await
        .expect("turn is archived");
    let turn = TurnId(receipt.turn_id);
    fixture
        .archive
        .memory_repository()
        .forget_source(receipt.source_id.expect("an eligible turn has a source"))
        .await
        .expect("source is forgotten");

    let examination = fixture
        .facts
        .finish_examination(turn, strings(&["The user's PIN is 4321."]))
        .await
        .expect("finish runs");

    assert!(examination.noted.is_empty());
    assert!(fixture.texts().await.is_empty());
}

#[tokio::test]
async fn a_turn_is_examined_at_most_once() {
    let fixture = Fixture::open("once").await;
    let turn = fixture.completed("My sister is called Mia.").await;

    let first = fixture
        .facts
        .finish_examination(turn, strings(&["Darren's sister is called Mia."]))
        .await
        .expect("first finish");
    assert_eq!(first.noted.len(), 1);
    assert_eq!(fixture.next_text(&[]).await, None);

    // A second finish for the same turn (a duplicate pass) changes nothing, even with
    // a different candidate.
    let second = fixture
        .facts
        .finish_examination(turn, strings(&["Darren has a brother."]))
        .await
        .expect("second finish");
    assert!(second.noted.is_empty());
    assert_eq!(fixture.texts().await, ["Darren's sister is called Mia."]);
}

#[tokio::test]
async fn candidates_are_held_to_the_fact_limits_and_duplicates_are_skipped() {
    let fixture = Fixture::open("candidates").await;
    fixture
        .facts
        .insert("My sister is called Mia", FactOrigin::Stated)
        .await
        .expect("stated fact is added");
    let turn = fixture.completed("lots of talk").await;
    let too_long = "x".repeat(201);

    let examination = fixture
        .facts
        .finish_examination(
            turn,
            vec![
                String::new(),
                "  ".to_owned(),
                "two\nlines".to_owned(),
                too_long,
                "  Darren likes tea  ".to_owned(),
                // The same as an existing fact, as a model repeats it back.
                "  MY SISTER IS CALLED MIA ".to_owned(),
                // The same as a fact this very pass just added.
                "darren likes TEA".to_owned(),
                "Darren lives in Leeds".to_owned(),
            ],
        )
        .await
        .expect("finish runs");

    let noted: Vec<&str> = examination
        .noted
        .iter()
        .map(|fact| fact.text.as_str())
        .collect();
    assert_eq!(noted, ["Darren likes tea", "Darren lives in Leeds"]);
    assert!(examination.dropped.is_empty());
    assert_eq!(
        fixture.texts().await,
        [
            "My sister is called Mia",
            "Darren likes tea",
            "Darren lives in Leeds"
        ]
    );
    // Whatever was refused, the turn counts as examined.
    assert_eq!(fixture.next_text(&[]).await, None);
}

#[tokio::test]
async fn a_full_list_drops_older_noted_facts_for_a_new_one_and_says_which() {
    let fixture = Fixture::open("full-list").await;
    // Ten noted facts of 100 bytes take 1,030 of the 1,200 bytes of the list.
    for index in 0..10 {
        let text = format!("{index}{}", "n".repeat(99));
        fixture
            .facts
            .insert(&text, FactOrigin::Noted)
            .await
            .expect("fixture fact fits");
    }
    let turn = fixture.completed("some talk").await;
    let first = format!("a{}", "a".repeat(149));
    let second = format!("b{}", "b".repeat(149));

    let examination = fixture
        .facts
        .finish_examination(turn, vec![first.clone(), second.clone()])
        .await
        .expect("finish runs");

    // 1,030 + 153 fits; the second makes 1,336, so the two oldest noted go (206 bytes).
    let noted: Vec<&str> = examination
        .noted
        .iter()
        .map(|fact| fact.text.as_str())
        .collect();
    assert_eq!(noted, [first.as_str(), second.as_str()]);
    let dropped: Vec<char> = examination
        .dropped
        .iter()
        .filter_map(|fact| fact.text.chars().next())
        .collect();
    assert_eq!(dropped, ['0', '1']);
    assert_eq!(fixture.texts().await.len(), 10);
}

#[tokio::test]
async fn a_fact_added_and_dropped_in_the_same_pass_is_not_reported() {
    let fixture = Fixture::open("same-pass").await;
    // Five stated facts of 197 bytes take 1,000 bytes, leaving room for one more line
    // of 200 bytes and no more.
    for index in 0..5 {
        let text = format!("{index}{}", "s".repeat(196));
        fixture
            .facts
            .insert(&text, FactOrigin::Stated)
            .await
            .expect("fixture fact fits");
    }
    let turn = fixture.completed("some talk").await;
    let first = "f".repeat(190);
    let second = "g".repeat(190);

    let examination = fixture
        .facts
        .finish_examination(turn, vec![first, second.clone()])
        .await
        .expect("finish runs");

    let noted: Vec<&str> = examination
        .noted
        .iter()
        .map(|fact| fact.text.as_str())
        .collect();
    assert_eq!(noted, [second.as_str()]);
    assert!(
        examination.dropped.is_empty(),
        "the first fact was never shown, so its drop is not news"
    );
    assert_eq!(fixture.texts().await.len(), 6);
}

#[tokio::test]
async fn a_full_list_of_stated_facts_refuses_candidates_but_the_turn_is_still_examined() {
    let fixture = Fixture::open("stated-full").await;
    for index in 0..6 {
        let text = format!("{index}{}", "s".repeat(196));
        fixture
            .facts
            .insert(&text, FactOrigin::Stated)
            .await
            .expect("fixture fact fits");
    }
    let turn = fixture.completed("some talk").await;

    let examination = fixture
        .facts
        .finish_examination(turn, strings(&["Darren likes tea"]))
        .await
        .expect("finish runs");

    assert!(examination.noted.is_empty());
    assert_eq!(fixture.texts().await.len(), 6);
    assert_eq!(fixture.next_text(&[]).await, None);
}
