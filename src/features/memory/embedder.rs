use std::{
    path::PathBuf,
    sync::{Arc, OnceLock},
    time::Duration,
};

use tokio::sync::Semaphore;

use crate::clients::local_embeddings::{LocalEmbeddings, LocalEmbeddingsError, MODEL_FINGERPRINT};

/// Native inference cannot be cancelled once started; this bounds how long a
/// caller waits for one result, not how long the worker thread runs.
const EMBED_TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum EmbedderState {
    Loading,
    Ready,
    Unavailable,
}

#[derive(Clone, Copy, Debug)]
pub(super) enum EmbedKind {
    Query,
    Document,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum EmbedError {
    #[error("the local embedding model is still loading")]
    Loading,
    #[error("the local embedding model is unavailable")]
    Unavailable,
    #[error("the local embedding worker is busy")]
    Busy,
    #[error("the local embedding worker did not answer in time")]
    TimedOut,
    #[error("local embedding failed")]
    Failed(#[source] LocalEmbeddingsError),
    #[error("local embedding task failed")]
    Task(#[source] tokio::task::JoinError),
}

/// Loads the local model in the background and runs one native inference at a
/// time. A single semaphore permit is held by whichever blocking task is
/// running (load or embed), so an abandoned wait never starts a second task
/// beside a thread that is still computing.
pub(crate) struct Embedder {
    model: Arc<OnceLock<Option<LocalEmbeddings>>>,
    worker: Arc<Semaphore>,
}

impl Embedder {
    pub(crate) fn load(model_dir: PathBuf) -> Self {
        let model = Arc::new(OnceLock::new());
        let worker = Arc::new(Semaphore::new(1));
        let permit = Arc::clone(&worker)
            .try_acquire_owned()
            .expect("a new semaphore has its permit");
        let loaded = Arc::clone(&model);
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let _ = loaded.set(LocalEmbeddings::load_from_dir(&model_dir).ok());
        });
        Self { model, worker }
    }

    /// Identifies the model that produced stored vectors; a change re-embeds.
    pub(crate) fn fingerprint(&self) -> &'static str {
        MODEL_FINGERPRINT
    }

    pub(crate) fn state(&self) -> EmbedderState {
        match self.model.get() {
            None => EmbedderState::Loading,
            Some(Some(_)) => EmbedderState::Ready,
            Some(None) => EmbedderState::Unavailable,
        }
    }

    /// Embeds `text`, waiting up to `queue_wait` for the worker to be free.
    pub(super) async fn embed(
        &self,
        text: String,
        kind: EmbedKind,
        queue_wait: Duration,
    ) -> Result<Vec<f32>, EmbedError> {
        match self.state() {
            EmbedderState::Loading => return Err(EmbedError::Loading),
            EmbedderState::Unavailable => return Err(EmbedError::Unavailable),
            EmbedderState::Ready => {}
        }
        let permit = tokio::time::timeout(queue_wait, Arc::clone(&self.worker).acquire_owned())
            .await
            .map_err(|_| EmbedError::Busy)?
            .expect("the embedder semaphore is never closed");
        let model = Arc::clone(&self.model);
        let task = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let model = model
                .get()
                .and_then(Option::as_ref)
                .expect("state was Ready before the task started");
            match kind {
                EmbedKind::Query => model.embed_query(&text),
                EmbedKind::Document => model.embed_document(&text),
            }
        });
        tokio::time::timeout(EMBED_TIMEOUT, task)
            .await
            .map_err(|_| EmbedError::TimedOut)?
            .map_err(EmbedError::Task)?
            .map_err(EmbedError::Failed)
    }

    /// Resolves once no native task holds the worker.
    pub(crate) async fn wait_until_idle(&self) {
        let _ = self.worker.acquire().await;
    }
}
