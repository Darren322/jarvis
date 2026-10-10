use std::{sync::mpsc, time::Duration};

use rig_core::{client::CompletionClient, providers::openai::CompletionsClient};
use tokio::task::JoinHandle;

use crate::{
    clients::local_embeddings::{LocalEmbeddings, LocalEmbeddingsError},
    storage::ConversationArchive,
};

use super::MemoryService;

async fn service_fixture() -> (tempfile::TempDir, MemoryService) {
    let directory = tempfile::tempdir().expect("temporary memory-service directory");
    let archive = ConversationArchive::open(directory.path().join("archive.sqlite3"))
        .await
        .expect("SQLite archive opens");
    let client = CompletionsClient::builder()
        .api_key("local")
        .base_url("http://127.0.0.1:11434/v1")
        .build()
        .expect("test completion client builds without a request");

    let service = MemoryService {
        repository: archive.memory_repository(),
        completion_model: client.completion_model("test-model"),
        embeddings: None,
        chunker: None,
        index: None,
        index_dir: directory.path().join("index"),
        warning: Some(
            "Local embedding model is still loading; recall is temporarily unavailable.".to_owned(),
        ),
        chunking_warning: false,
        active_model_load: None,
        active_embedding: None,
        active_job: None,
        extraction_started: false,
        active_projection: None,
    };
    (directory, service)
}

fn blocked_model_load() -> (
    JoinHandle<Result<LocalEmbeddings, LocalEmbeddingsError>>,
    tokio::sync::oneshot::Receiver<()>,
    mpsc::Sender<()>,
) {
    let (started_sender, started) = tokio::sync::oneshot::channel();
    let (release, release_receiver) = mpsc::channel();
    let handle = tokio::task::spawn_blocking(move || {
        let _ = started_sender.send(());
        let _ = release_receiver.recv();
        Err(LocalEmbeddingsError::RuntimeNotConfigured)
    });
    (handle, started, release)
}

#[tokio::test]
async fn status_is_nonblocking_and_model_load_results_are_settled_once() {
    let (_directory, mut service) = service_fixture().await;
    let (handle, started, release) = blocked_model_load();
    service.active_model_load = Some(handle);
    started.await.expect("blocking model-load fixture started");

    let status = tokio::time::timeout(Duration::from_secs(2), service.status())
        .await
        .expect("status must not wait for unfinished native model loading")
        .expect("SQLite status is available");
    assert!(!status.backend_available);
    assert!(
        status
            .warning
            .as_deref()
            .is_some_and(|warning| { warning.contains("still loading") })
    );
    assert!(service.active_model_load.is_some());

    release.send(()).expect("release model-load fixture");
    while service
        .active_model_load
        .as_ref()
        .is_some_and(|worker| !worker.is_finished())
    {
        tokio::task::yield_now().await;
    }

    // Status itself must take ownership of a completed load and refresh the warning.
    let status = service
        .status()
        .await
        .expect("status settles a completed model load");
    assert!(service.active_model_load.is_none());
    assert!(
        status
            .warning
            .as_deref()
            .is_some_and(|warning| { warning.contains("model unavailable") })
    );

    // A separately awaited load exercises the wait→status path and protects
    // against awaiting the same completed JoinHandle twice.
    let (handle, started, release) = blocked_model_load();
    service.active_model_load = Some(handle);
    started.await.expect("second model-load fixture started");
    release.send(()).expect("release second model-load fixture");
    service.wait_for_native_worker().await;
    assert!(service.active_model_load.is_none());
    let status = service
        .status()
        .await
        .expect("status follows the settled wait without polling twice");
    assert!(
        status
            .warning
            .as_deref()
            .is_some_and(|warning| { warning.contains("model unavailable") })
    );
}

#[tokio::test]
async fn cancelling_native_worker_wait_keeps_embedding_handle_owned() {
    let (_directory, mut service) = service_fixture().await;
    let (started_sender, started) = tokio::sync::oneshot::channel();
    let (release, release_receiver) = mpsc::channel();
    service.active_embedding = Some(tokio::task::spawn_blocking(move || {
        let _ = started_sender.send(());
        let _ = release_receiver.recv();
        Ok(vec![1.0_f32])
    }));
    started.await.expect("blocking embedding fixture started");

    {
        let wait = service.wait_for_native_worker();
        tokio::pin!(wait);
        tokio::select! {
            biased;
            _ = &mut wait => panic!("wait completed before the controlled worker was released"),
            _ = tokio::task::yield_now() => {}
        }
    }
    assert!(service.active_embedding.is_some());
    assert!(
        service
            .active_embedding
            .as_ref()
            .is_some_and(|worker| !worker.is_finished())
    );

    release.send(()).expect("release embedding fixture");
    service.wait_for_native_worker().await;
    assert!(service.active_embedding.is_none());
    service
        .status()
        .await
        .expect("status can follow a completed and reaped worker");
}
