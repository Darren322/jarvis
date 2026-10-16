use std::path::PathBuf;

use serde_json::json;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

use crate::clients::remote_embeddings::RemoteEmbeddings;
use crate::config::EmbeddingConfig;
use crate::storage::{ArchiveOutcome, ConversationArchive, MemoryEligibility, SourceId};

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

/// An embedding server whose health check fails; `/v1/embeddings` answers
/// with `embeddings_status` and a unit vector.
async fn embedding_server(embeddings_status: u16) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;
    let mut vector = vec![0.0_f32; 384];
    vector[0] = 1.0;
    Mock::given(method("POST"))
        .and(path("/v1/embeddings"))
        .respond_with(
            ResponseTemplate::new(embeddings_status)
                .set_body_json(json!({"data": [{"embedding": vector}]})),
        )
        .mount(&server)
        .await;
    server
}

/// Archives one remembered turn and opens memory against `server`.
async fn open_with_one_memory(
    label: &str,
    server: &MockServer,
) -> (PathBuf, ConversationArchive, MemoryService, SourceId) {
    let path = database_path(label);
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
    let embeddings = RemoteEmbeddings::new(&EmbeddingConfig {
        base_url: format!("{}/v1", server.uri()),
        model: "bge-test".to_owned(),
    })
    .expect("a localhost URL is allowed");
    let memory = MemoryService::open(archive.memory_repository(), embeddings);
    memory.wait_until_embedder_idle().await;
    (path, archive, memory, cat)
}

#[tokio::test]
async fn recall_falls_back_to_keywords_when_the_embedder_is_unavailable() {
    let server = embedding_server(503).await;
    let (path, archive, memory, cat) = open_with_one_memory("keyword-fallback", &server).await;
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
    assert_eq!(
        memory.status().await.expect("status").embedder,
        EmbedderState::Unavailable
    );

    let excluded = memory.recall("What is my cat called?", &[cat]).await;
    assert!(excluded.documents.is_empty());

    let pass = memory.run_idle_pass().await.expect("idle pass runs");
    assert!(!pass.has_made_progress());

    drop(memory);
    drop(archive);
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn a_successful_recall_marks_the_server_ready_and_resumes_idle_embedding() {
    let server = embedding_server(200).await;
    let (path, archive, memory, cat) = open_with_one_memory("server-recovers", &server).await;
    assert_eq!(
        memory.status().await.expect("status").embedder,
        EmbedderState::Unavailable
    );

    let recall = memory.recall("What is my cat called?", &[]).await;
    assert_eq!(recall.warning, None);
    assert_eq!(recall.source_ids, vec![cat]);
    assert_eq!(
        memory.status().await.expect("status").embedder,
        EmbedderState::Ready
    );

    let pass = memory.run_idle_pass().await.expect("idle pass runs");
    assert!(pass.has_made_progress());
    assert_eq!(
        memory
            .status()
            .await
            .expect("status")
            .stats
            .pending_embeddings,
        0
    );

    drop(memory);
    drop(archive);
    let _ = std::fs::remove_file(&path);
}
