use std::{error::Error, path::PathBuf};

use crate::storage::{
    ArchiveOutcome, ConversationArchive, ImportState, MemoryEligibility, MemoryId, MemoryKind,
    MemoryRepository, MemorySaveState, SourceId, StorageError,
};

const MODEL: &str = "test-model";
const OTHER_MODEL: &str = "other-model";

fn database_path(label: &str) -> PathBuf {
    let name = format!(
        "jarvis-storage-memory-{label}-{}-{}.sqlite3",
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
    repository: MemoryRepository,
}

impl Fixture {
    async fn open(label: &str) -> Self {
        let path = database_path(label);
        let archive = ConversationArchive::open(&path)
            .await
            .expect("archive opens");
        let repository = archive.memory_repository();
        Self {
            path,
            archive,
            repository,
        }
    }

    async fn remember(&self, user: &str, answer: &str, recalled: &[SourceId]) -> SourceId {
        let receipt = self
            .archive
            .append_turn_with_memory_and_sources(
                user,
                ArchiveOutcome::Completed(answer.to_owned()),
                MemoryEligibility::Eligible,
                recalled,
            )
            .await
            .expect("turn is archived");
        assert_eq!(receipt.memory, MemorySaveState::Saved);
        receipt.source_id.expect("eligible turn has a source")
    }

    async fn keyword(&self, query: &str) -> Vec<MemoryId> {
        self.repository
            .search_keyword(query.to_owned(), 10)
            .await
            .expect("keyword search runs")
    }

    fn sql_count(&self, sql: &str) -> i64 {
        let connection = rusqlite::Connection::open(&self.path).expect("fixture database opens");
        connection
            .query_row(sql, [], |row| row.get(0))
            .expect("count query runs")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn unit_vector(axis: usize) -> Vec<f32> {
    let mut vector = vec![0.0; 384];
    vector[axis] = 1.0;
    vector
}

#[tokio::test]
async fn completed_turns_become_keyword_searchable_and_failed_turns_do_not() {
    let fixture = Fixture::open("keyword").await;
    fixture
        .remember("My cat is called Miso.", "Noted, master.", &[])
        .await;
    let failed = fixture
        .archive
        .append_turn_with_memory_and_sources(
            "Remember the gate code 1234",
            ArchiveOutcome::TimedOut,
            MemoryEligibility::Eligible,
            &[],
        )
        .await
        .expect("failed turn is archived");
    assert_eq!(failed.memory, MemorySaveState::NotEligible);
    assert_eq!(failed.source_id, None);

    // Porter stemming matches "cats" to "cat".
    assert_eq!(fixture.keyword("\"cats\"").await.len(), 1);
    assert!(fixture.keyword("\"gate\"").await.is_empty());

    let stats = fixture.repository.stats(MODEL).await.expect("stats load");
    assert_eq!(stats.memories, 1);
    assert_eq!(stats.pending_embeddings, 1);
    assert_eq!(stats.conversation_sources, 1);
}

#[tokio::test]
async fn long_turns_are_split_into_bounded_chunks() {
    let fixture = Fixture::open("chunks").await;
    let answer = "This sentence fills the answer. ".repeat(120);
    let source = fixture
        .remember("Tell me a long story.", &answer, &[])
        .await;

    let memories = fixture
        .repository
        .list_memories(None, 50)
        .await
        .expect("memories list");
    assert!(memories.len() >= 3);
    assert!(memories.iter().all(|memory| memory.source_id == source
        && memory.kind == MemoryKind::Excerpt
        && memory.text.len() <= 1500));
}

#[tokio::test]
async fn vectors_are_stored_searched_and_scoped_to_their_model() {
    let fixture = Fixture::open("vectors").await;
    fixture.remember("I like hiking.", "Noted.", &[]).await;
    fixture.remember("My cat is Miso.", "Noted.", &[]).await;
    let pending = fixture
        .repository
        .pending_embeddings(MODEL, 10)
        .await
        .expect("pending memories load");
    assert_eq!(pending.len(), 2);
    for (axis, (id, _)) in pending.iter().enumerate() {
        assert!(
            fixture
                .repository
                .store_embedding(*id, MODEL, unit_vector(axis))
                .await
                .expect("vector stores")
        );
    }
    assert!(
        fixture
            .repository
            .pending_embeddings(MODEL, 10)
            .await
            .expect("pending memories load")
            .is_empty()
    );

    let hits = fixture
        .repository
        .search_vector(unit_vector(1), MODEL, 10)
        .await
        .expect("vector search runs");
    assert_eq!(hits[0].0, pending[1].0);
    assert!((hits[0].1 - 1.0).abs() < 1e-5);
    assert!(hits[1].1.abs() < 1e-5);

    // Another model's vectors are not comparable and count as pending.
    assert!(
        fixture
            .repository
            .search_vector(unit_vector(1), OTHER_MODEL, 10)
            .await
            .expect("vector search runs")
            .is_empty()
    );
    assert_eq!(
        fixture
            .repository
            .stats(OTHER_MODEL)
            .await
            .expect("stats load")
            .pending_embeddings,
        2
    );

    assert_eq!(
        fixture.repository.reset_embeddings().await.expect("reset"),
        2
    );
    assert_eq!(fixture.sql_count("SELECT COUNT(*) FROM memory_vectors"), 0);
}

#[tokio::test]
async fn rejects_wrong_dimensions_and_skips_deleted_memories() {
    let fixture = Fixture::open("dimensions").await;
    let source = fixture.remember("Remember this.", "Done.", &[]).await;
    let (id, _) = fixture
        .repository
        .pending_embeddings(MODEL, 1)
        .await
        .expect("pending")[0]
        .clone();
    assert!(matches!(
        fixture
            .repository
            .store_embedding(id, MODEL, vec![1.0; 3])
            .await,
        Err(StorageError::InvalidInput(_))
    ));

    fixture
        .repository
        .forget_source(source)
        .await
        .expect("forget runs");
    assert!(
        !fixture
            .repository
            .store_embedding(id, MODEL, unit_vector(0))
            .await
            .expect("store runs")
    );
    assert_eq!(fixture.sql_count("SELECT COUNT(*) FROM memory_vectors"), 0);
}

#[tokio::test]
async fn forgetting_cascades_to_answers_that_recalled_the_source() {
    let fixture = Fixture::open("cascade").await;
    let cat = fixture.remember("My cat is Miso.", "Noted.", &[]).await;
    let derived = fixture
        .remember("What is my cat called?", "Your cat Miso.", &[cat])
        .await;
    let unrelated = fixture.remember("I like hiking.", "Noted.", &[]).await;
    let (cat_memory, _) = fixture
        .repository
        .pending_embeddings(MODEL, 1)
        .await
        .expect("pending")[0]
        .clone();
    fixture
        .repository
        .store_embedding(cat_memory, MODEL, unit_vector(0))
        .await
        .expect("vector stores");

    let receipt = fixture
        .repository
        .forget_memory(cat_memory)
        .await
        .expect("forget runs");
    assert_eq!(receipt.forgotten_sources, 2);
    assert_eq!(receipt.forgotten_memories, 2);
    assert_eq!(receipt.affected_turns, 2);

    assert!(fixture.keyword("\"miso\"").await.is_empty());
    assert_eq!(fixture.keyword("\"hiking\"").await.len(), 1);
    assert_eq!(fixture.sql_count("SELECT COUNT(*) FROM memory_vectors"), 0);
    assert_eq!(
        fixture.sql_count("SELECT COUNT(*) FROM sources WHERE state = 'forgotten'"),
        2
    );
    // Archived turns themselves are kept.
    assert_eq!(fixture.sql_count("SELECT COUNT(*) FROM turns"), 3);

    let again = fixture
        .repository
        .forget_source(cat)
        .await
        .expect("forget runs");
    assert!(again.is_empty());
    let _ = (derived, unrelated);
}

#[tokio::test]
async fn an_answer_built_on_a_forgotten_source_is_not_remembered() {
    let fixture = Fixture::open("suppressed").await;
    let cat = fixture.remember("My cat is Miso.", "Noted.", &[]).await;
    fixture
        .repository
        .forget_source(cat)
        .await
        .expect("forget runs");

    let receipt = fixture
        .archive
        .append_turn_with_memory_and_sources(
            "What is my cat called?",
            ArchiveOutcome::Completed("Miso.".to_owned()),
            MemoryEligibility::Eligible,
            &[cat],
        )
        .await
        .expect("turn is archived");
    assert_eq!(receipt.memory, MemorySaveState::Suppressed);
    assert_eq!(fixture.sql_count("SELECT COUNT(*) FROM memories"), 0);

    let unknown = fixture
        .archive
        .append_turn_with_memory_and_sources(
            "Hello",
            ArchiveOutcome::Completed("Hi.".to_owned()),
            MemoryEligibility::Eligible,
            &[SourceId(9_999)],
        )
        .await
        .expect("turn is archived");
    assert_eq!(unknown.memory, MemorySaveState::Suppressed);
}

#[tokio::test]
async fn imports_track_revisions_and_forgotten_revisions_stay_forgotten() {
    let fixture = Fixture::open("imports").await;
    let import = |text: &str| {
        fixture.repository.import_source(
            "/notes/garden.md".to_owned(),
            "garden.md".to_owned(),
            text.to_owned(),
        )
    };

    let first = import("Tomatoes need sun.").await.expect("import runs");
    assert_eq!(first.state, ImportState::Created);
    assert_eq!(first.chunks, 1);
    assert_eq!(
        import("Tomatoes need sun.")
            .await
            .expect("import runs")
            .state,
        ImportState::AlreadyCurrent
    );

    let second = import("Basil needs water.").await.expect("import runs");
    assert_eq!(second.state, ImportState::Created);
    assert!(fixture.keyword("\"tomatoes\"").await.is_empty());
    assert_eq!(fixture.keyword("\"basil\"").await.len(), 1);

    let back = import("Tomatoes need sun.").await.expect("import runs");
    assert_eq!(back.state, ImportState::Reactivated);
    assert_eq!(back.source_id, first.source_id);
    assert_eq!(fixture.keyword("\"tomatoes\"").await.len(), 1);
    assert!(fixture.keyword("\"basil\"").await.is_empty());

    fixture
        .repository
        .forget_source(first.source_id)
        .await
        .expect("forget runs");
    assert_eq!(
        import("Tomatoes need sun.")
            .await
            .expect("import runs")
            .state,
        ImportState::Forgotten
    );
    assert!(fixture.keyword("\"tomatoes\"").await.is_empty());
    assert!(matches!(
        import("   ").await,
        Err(StorageError::InvalidInput(_))
    ));
}

#[tokio::test]
async fn memory_text_is_immutable() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::open("immutable").await;
    fixture.remember("Remember this.", "Done.", &[]).await;
    let connection = rusqlite::Connection::open(&fixture.path)?;
    let update = connection.execute("UPDATE memories SET text = 'changed'", []);
    assert!(update.is_err());
    Ok(())
}
