use std::path::PathBuf;

use crate::storage::{ArchiveOutcome, ConversationArchive, MemoryEligibility};

use super::{EmbedderState, MemoryService};

fn database_path(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "jarvis-memory-service-{label}-{}-{}.sqlite3",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock is after Unix epoch")
            .as_nanos()
    ))
}

#[tokio::test]
async fn recall_falls_back_to_keywords_when_the_embedder_is_unavailable() {
    let path = database_path("keyword-fallback");
    let archive = ConversationArchive::open(&path)
        .await
        .expect("archive opens");
    let receipt = archive
        .append_turn(
            "My cat is called Miso.",
            ArchiveOutcome::Completed("Noted, master.".to_owned()),
            MemoryEligibility::Eligible,
            &[],
        )
        .await
        .expect("turn is archived");
    let cat = receipt.source_id.expect("eligible turn has a source");

    let memory = MemoryService::open(
        archive.memory_repository(),
        PathBuf::from("/nonexistent/jarvis-embeddings"),
    );
    memory.wait_until_embedder_idle().await;
    assert_eq!(
        memory.status().await.expect("status").embedder,
        EmbedderState::Unavailable
    );

    let recall = memory.recall("What is my cat called?", &[]).await;
    assert_eq!(recall.documents.len(), 1);
    assert!(recall.documents[0].text.contains("Miso"));
    assert_eq!(recall.source_ids, vec![cat]);
    assert!(
        recall
            .warning
            .expect("degraded recall warns")
            .contains("keyword-only")
    );

    let excluded = memory.recall("What is my cat called?", &[cat]).await;
    assert!(excluded.documents.is_empty());

    let pass = memory.run_idle_pass().await.expect("idle pass runs");
    assert!(!pass.has_made_progress());

    drop(memory);
    drop(archive);
    let _ = std::fs::remove_file(&path);
}
