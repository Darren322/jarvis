use tempfile::tempdir;

use super::*;
use crate::storage::SourceKind;

fn projection(id: i64, text: &str) -> ProjectionLease {
    ProjectionLease {
        id: MemoryId(id),
        source_id: crate::storage::SourceId(20 + id),
        source_revision_sha256: format!("revision-{id}"),
        source_kind: SourceKind::Conversation,
        source_name: None,
        created_at_unix_ms: 1_800_000_000_000,
        generation: 1,
        lease_token: 1,
        operation: ProjectionOperation::Upsert,
        text: text.to_owned(),
        kind: MemoryRecordKind::Preference,
        attribution: MemoryAttribution::UserStatement,
    }
}

fn vector(primary: usize) -> Vec<f32> {
    let mut vector = vec![0.0; 384];
    vector[primary] = 1.0;
    vector
}

#[tokio::test]
async fn empty_production_table_accepts_upserts_and_native_hybrid_search_after_reopen() {
    let directory = tempdir().expect("temporary LanceDB directory");
    let mut index = MemoryIndex::open_rebuildable(directory.path())
        .await
        .expect("production starts with an empty table and installs FTS");

    let lexical = projection(1, "My dog's name is Miso; backup tag XQ-814.");
    let semantic = projection(2, "The canine companion answers to Miso.");
    let unrelated = projection(3, "My favorite project name is Asana.");
    index.apply(&lexical, Some(vector(1))).await.unwrap();
    index.apply(&semantic, Some(vector(0))).await.unwrap();
    index.apply(&unrelated, Some(vector(2))).await.unwrap();
    // Merge-insert is idempotent for a retried durable projection lease.
    index.apply(&lexical, Some(vector(1))).await.unwrap();

    let query_vector = vector(0);
    let candidates = index
        .search("dog name Miso", &query_vector)
        .await
        .expect("native vector, FTS, and hybrid search");
    assert!(candidates.lexical.contains(&MemoryId(1)));
    assert!(candidates.cosine.contains_key(&MemoryId(2)));
    assert!(candidates.fused.contains(&MemoryId(1)));
    assert!(candidates.fused.contains(&MemoryId(2)));
    assert!(
        !candidates.lexical.contains(&MemoryId(3)),
        "FTS AND query returned unrelated project row among {:?}",
        candidates.lexical
    );

    let exact_identifier = index
        .search("XQ-814", &vector(3))
        .await
        .expect("rare exact identifier query");
    assert!(exact_identifier.lexical.contains(&MemoryId(1)));

    index.delete(MemoryId(1)).await.unwrap();
    drop(index);
    index = MemoryIndex::open_rebuildable(directory.path())
        .await
        .expect("reopen the persisted index after delete");
    let reopened = index
        .search("dog name Miso", &query_vector)
        .await
        .expect("native search after reopen");
    assert!(!reopened.fused.contains(&MemoryId(1)));
    assert!(reopened.fused.contains(&MemoryId(2)));

    let unrelated = index
        .search("unknown dog name", &vector(3))
        .await
        .expect("unrelated native query");
    assert!(
        unrelated.lexical.is_empty(),
        "unrelated FTS candidate ids: {:?}",
        unrelated.lexical
    );
    assert!(
        unrelated
            .cosine
            .values()
            .all(|cosine| *cosine < super::super::recall::BGE_MIN_COSINE)
    );
}

#[tokio::test]
async fn failed_open_does_not_move_an_unmarked_user_path() {
    let directory = tempdir().expect("temporary parent directory");
    let index_path = directory.path().join("project-root");
    std::fs::write(&index_path, "user data").expect("create unrelated user file");

    assert!(MemoryIndex::open_rebuildable(&index_path).await.is_err());
    assert_eq!(std::fs::read(&index_path).unwrap(), b"user data");
}

#[tokio::test]
async fn opening_an_unmarked_existing_directory_preserves_unrelated_files() {
    let directory = tempdir().expect("temporary parent directory");
    let index_path = directory.path().join("project-root");
    std::fs::create_dir_all(&index_path).expect("create unrelated project directory");
    let unrelated = index_path.join("keep-me.txt");
    std::fs::write(&unrelated, "unrelated project data").expect("write unrelated file");

    assert!(MemoryIndex::open_rebuildable(&index_path).await.is_err());
    assert_eq!(
        std::fs::read(&unrelated).unwrap(),
        b"unrelated project data"
    );
    assert!(index_path.is_dir());
}

#[tokio::test]
async fn valid_unmarked_lance_projection_is_preserved_and_refused() {
    let directory = tempdir().expect("temporary parent directory");
    let index_path = directory.path().join("user-lancedb");
    let existing = MemoryIndex::open(&index_path)
        .await
        .expect("create a valid unmarked Lance table");
    drop(existing);

    assert!(matches!(
        MemoryIndex::open_rebuildable(&index_path).await,
        Err(IndexError::UnownedPath)
    ));
    let preserved = MemoryIndex::open(&index_path)
        .await
        .expect("the existing user table remains readable");
    assert!(!preserved.was_created);
}
