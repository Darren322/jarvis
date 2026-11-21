use std::{error::Error, path::PathBuf};

use super::{AddedFact, FACT_LINE_OVERHEAD_BYTES, MAX_FACTS_BYTES};
use crate::storage::{
    ArchiveOutcome, ConversationArchive, Fact, FactError, FactId, FactOrigin, FactsRepository,
    MAX_FACT_BYTES, MemoryEligibility, MemoryRepository,
};

use FactOrigin::{Noted, Stated};

fn database_path(label: &str) -> PathBuf {
    let name = format!(
        "jarvis-storage-facts-{label}-{}-{}.sqlite3",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock is after Unix epoch")
            .as_nanos()
    );
    std::env::temp_dir().join(name)
}

struct Fixture {
    path: PathBuf,
    archive: ConversationArchive,
    facts: FactsRepository,
}

impl Fixture {
    async fn open(label: &str) -> Self {
        let path = database_path(label);
        let archive = ConversationArchive::open(&path)
            .await
            .expect("archive opens");
        let facts = archive.facts_repository();
        Self {
            path,
            archive,
            facts,
        }
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

    /// Inserts `(origin, text bytes)` facts in order, each text unique and of the
    /// stated length, and returns their ids.
    async fn fill(&self, facts: &[(FactOrigin, usize)]) -> Vec<FactId> {
        let mut ids = Vec::new();
        for (index, (origin, bytes)) in facts.iter().enumerate() {
            let added = self
                .facts
                .insert(&text_of(index, *bytes), *origin)
                .await
                .expect("the fixture fact fits");
            assert!(added.dropped.is_empty(), "the fixture list is not full yet");
            ids.push(added.id);
        }
        ids
    }

    fn memory_repository(&self) -> MemoryRepository {
        self.archive.memory_repository()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// A text of exactly `bytes` ASCII bytes, starting with a tag that tells facts apart.
fn text_of(tag: usize, bytes: usize) -> String {
    let mut text = format!("f{tag}-");
    assert!(bytes >= text.len(), "text is too short for its tag");
    text.push_str(&"x".repeat(bytes - text.len()));
    text
}

#[tokio::test]
async fn facts_are_inserted_listed_by_id_and_removed() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::open("crud").await;
    assert!(fixture.facts.list().await?.is_empty());

    let first: AddedFact = fixture
        .facts
        .insert("  My sister is Mia.  ", Stated)
        .await?;
    let second = fixture.facts.insert("I like oolong tea.", Noted).await?;
    let third = fixture.facts.insert("I wake at six.", Stated).await?;
    assert!(first.id < second.id && second.id < third.id);

    let listed = fixture.facts.list().await?;
    let summary: Vec<_> = listed
        .iter()
        .map(|fact| (fact.id, fact.text.as_str(), fact.origin))
        .collect();
    assert_eq!(
        summary,
        [
            (first.id, "My sister is Mia.", Stated),
            (second.id, "I like oolong tea.", Noted),
            (third.id, "I wake at six.", Stated),
        ],
        "oldest first, text trimmed, origin kept"
    );

    let removed = fixture.facts.remove(second.id).await?;
    assert_eq!(
        removed.map(|fact| fact.text),
        Some("I like oolong tea.".to_owned())
    );
    assert_eq!(
        fixture.texts().await,
        ["My sister is Mia.", "I wake at six."]
    );
    assert_eq!(fixture.facts.remove(second.id).await?, None);

    // A removed fact's id is never given to a later fact, so a stale "forget"
    // cannot remove the wrong one.
    let fourth = fixture.facts.insert("I have a cat.", Noted).await?;
    assert!(fourth.id > third.id);
    Ok(())
}

#[tokio::test]
async fn a_fact_survives_reopening_the_database() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::open("reopen").await;
    fixture.facts.insert("My sister is Mia.", Stated).await?;

    let reopened = ConversationArchive::open(&fixture.path).await?;
    let listed = reopened.facts_repository().list().await?;

    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].origin, Stated);
    assert!(listed[0].created_at_unix_ms > 0);
    Ok(())
}

#[tokio::test]
async fn the_per_fact_limit_counts_bytes_and_text_must_be_one_line() {
    struct Case {
        name: &'static str,
        text: String,
        accepted: bool,
    }
    let multibyte = |ascii: usize| format!("{}é", "a".repeat(ascii));
    let cases = [
        Case {
            name: "the last accepted size",
            text: text_of(0, MAX_FACT_BYTES),
            accepted: true,
        },
        Case {
            name: "one byte more",
            text: text_of(0, MAX_FACT_BYTES + 1),
            accepted: false,
        },
        Case {
            name: "multibyte text at the limit (199 ascii bytes and a 2-byte letter)",
            text: multibyte(MAX_FACT_BYTES - 2),
            accepted: true,
        },
        Case {
            name: "multibyte text one byte over, though it is only 200 characters",
            text: multibyte(MAX_FACT_BYTES - 1),
            accepted: false,
        },
        Case {
            name: "empty",
            text: String::new(),
            accepted: false,
        },
        Case {
            name: "only whitespace",
            text: " \t ".to_owned(),
            accepted: false,
        },
        Case {
            name: "two lines",
            text: "My sister is Mia.\nShe lives in Leeds.".to_owned(),
            accepted: false,
        },
        Case {
            name: "a trailing newline is trimmed, not refused",
            text: "My sister is Mia.\n".to_owned(),
            accepted: true,
        },
    ];

    for case in cases {
        let fixture = Fixture::open("limit").await;
        let result = fixture.facts.insert(&case.text, Stated).await;
        assert_eq!(result.is_ok(), case.accepted, "{}", case.name);
        assert_eq!(
            fixture.texts().await.len(),
            usize::from(case.accepted),
            "{}",
            case.name
        );
    }
}

#[tokio::test]
async fn refusals_name_their_reason() {
    let fixture = Fixture::open("reasons").await;
    assert!(matches!(
        fixture.facts.insert("", Stated).await,
        Err(FactError::Empty)
    ));
    assert!(matches!(
        fixture.facts.insert("a\nb", Stated).await,
        Err(FactError::NotOneLine)
    ));
    assert!(matches!(
        fixture
            .facts
            .insert(&text_of(0, MAX_FACT_BYTES + 1), Stated)
            .await,
        Err(FactError::TooLong)
    ));
}

#[tokio::test]
async fn the_list_cap_is_enforced_at_its_exact_byte() {
    // Five stated facts of 197 bytes take 1,000 of the 1,200; the next fact's line
    // is its bytes plus the framing.
    let line = MAX_FACTS_BYTES / 6;
    let text_bytes = line - FACT_LINE_OVERHEAD_BYTES;
    assert_eq!(line * 6, MAX_FACTS_BYTES);
    for origin in [Stated, Noted] {
        for (name, last_bytes, accepted) in [
            ("the list reaches the cap exactly", text_bytes, true),
            ("one byte over the cap", text_bytes + 1, false),
        ] {
            let fixture = Fixture::open("cap").await;
            fixture.fill(&[(Stated, text_bytes); 5]).await;
            let before = fixture.texts().await;

            let result = fixture.facts.insert(&text_of(9, last_bytes), origin).await;

            let case = format!("{name} ({origin:?})");
            assert_eq!(result.is_ok(), accepted, "{case}");
            if accepted {
                assert_eq!(fixture.texts().await.len(), 6, "{case}");
                let total: usize = fixture
                    .facts
                    .list()
                    .await
                    .expect("facts list")
                    .iter()
                    .map(Fact::line_bytes)
                    .sum();
                assert_eq!(total, MAX_FACTS_BYTES, "{case}");
            } else {
                assert!(matches!(result, Err(FactError::ListFull)), "{case}");
                assert_eq!(fixture.texts().await, before, "{case}: nothing changed");
            }
        }
    }
}

#[tokio::test]
async fn a_noted_fact_is_dropped_for_a_new_one_oldest_first() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::open("drop-oldest").await;
    let full = 197;
    // stated, noted, noted, stated, noted, noted: six lines of 200 bytes, full.
    let ids = fixture
        .fill(&[
            (Stated, full),
            (Noted, full),
            (Noted, full),
            (Stated, full),
            (Noted, full),
            (Noted, full),
        ])
        .await;

    let added = fixture.facts.insert(&text_of(6, full), Stated).await?;
    assert_eq!(
        added.dropped.iter().map(|fact| fact.id).collect::<Vec<_>>(),
        [ids[1]],
        "the oldest noted fact goes, the stated one before it stays"
    );
    let added = fixture.facts.insert(&text_of(7, full), Noted).await?;
    assert_eq!(
        added.dropped.iter().map(|fact| fact.id).collect::<Vec<_>>(),
        [ids[2]],
        "a new noted fact also makes room from the oldest noted"
    );

    let remaining: Vec<_> = fixture
        .facts
        .list()
        .await?
        .into_iter()
        .map(|fact| fact.id)
        .collect();
    assert_eq!(remaining[..3], [ids[0], ids[3], ids[4]]);
    assert_eq!(remaining[3], ids[5]);
    assert_eq!(remaining.len(), 6);
    Ok(())
}

#[tokio::test]
async fn several_noted_facts_are_dropped_when_one_is_not_enough() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::open("drop-several").await;
    // Four stated facts take 800 bytes; four noted lines of 103, 103, 103 and 91
    // fill the other 400.
    let ids = fixture
        .fill(&[
            (Stated, 197),
            (Stated, 197),
            (Stated, 197),
            (Stated, 197),
            (Noted, 100),
            (Noted, 100),
            (Noted, 100),
            (Noted, 88),
        ])
        .await;

    // A 200-byte fact needs 203: the first noted line frees 103, the second 206.
    let added = fixture
        .facts
        .insert(&text_of(9, MAX_FACT_BYTES), Stated)
        .await?;

    assert_eq!(
        added.dropped.iter().map(|fact| fact.id).collect::<Vec<_>>(),
        [ids[4], ids[5]]
    );
    let listed: Vec<_> = fixture
        .facts
        .list()
        .await?
        .into_iter()
        .map(|fact| fact.id)
        .collect();
    assert_eq!(
        listed[..6],
        [ids[0], ids[1], ids[2], ids[3], ids[6], ids[7]]
    );
    assert_eq!(listed[6], added.id);
    Ok(())
}

#[tokio::test]
async fn stated_facts_are_never_dropped_and_a_refusal_drops_nothing() {
    // Stated facts alone fill the list: nothing can be dropped.
    for origin in [Stated, Noted] {
        let fixture = Fixture::open("stated-full").await;
        fixture.fill(&[(Stated, 197); 6]).await;
        let before = fixture.facts.list().await.expect("facts list");

        let result = fixture.facts.insert("one more", origin).await;

        assert!(matches!(result, Err(FactError::ListFull)), "{origin:?}");
        assert_eq!(fixture.facts.list().await.expect("facts list"), before);
    }

    // Dropping the only noted fact is not enough room: it must stay.
    let fixture = Fixture::open("not-enough").await;
    fixture
        .fill(&[
            (Stated, 197),
            (Stated, 197),
            (Stated, 197),
            (Stated, 197),
            (Stated, 197),
            (Noted, 197),
        ])
        .await;
    let before = fixture.facts.list().await.expect("facts list");

    // The new line is 203 bytes; the noted fact frees only 200.
    let result = fixture
        .facts
        .insert(&text_of(9, MAX_FACT_BYTES), Stated)
        .await;

    assert!(matches!(result, Err(FactError::ListFull)));
    assert_eq!(fixture.facts.list().await.expect("facts list"), before);
}

#[tokio::test]
async fn a_search_never_returns_a_fact() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::open("not-searchable").await;
    fixture
        .facts
        .insert("My cat is called Miso.", Stated)
        .await?;
    let memories = fixture.memory_repository();

    assert!(
        memories
            .search_keyword("miso".to_owned(), 10)
            .await?
            .is_empty()
    );
    assert!(memories.list_memories(None, 10).await?.is_empty());
    assert!(
        memories
            .list_memories(Some("miso".to_owned()), 10)
            .await?
            .is_empty()
    );

    // The same words in a conversation are found; the fact stays out of the way.
    fixture
        .archive
        .append_turn(
            "My cat is called Miso.",
            ArchiveOutcome::Completed("Noted, master.".to_owned()),
            MemoryEligibility::Eligible,
            &[],
        )
        .await?;
    let hits = memories.search_keyword("miso".to_owned(), 10).await?;
    assert_eq!(hits.len(), 1);
    Ok(())
}
