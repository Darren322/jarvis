use std::{
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    },
    time::Duration,
};

use tokio::sync::Semaphore;

use crate::clients::remote_embeddings::{
    MODEL_FINGERPRINT, RemoteEmbeddings, RemoteEmbeddingsError,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum EmbedderState {
    /// The startup health check has not answered yet.
    Connecting,
    Ready,
    /// The last request failed; recall still tries, idle work waits.
    Unavailable,
}

impl EmbedderState {
    fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::Connecting,
            1 => Self::Ready,
            _ => Self::Unavailable,
        }
    }

    fn as_u8(self) -> u8 {
        match self {
            Self::Connecting => 0,
            Self::Ready => 1,
            Self::Unavailable => 2,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) enum EmbedKind {
    Query,
    Document,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum EmbedError {
    #[error("another embedding request is still running")]
    Busy,
    #[error("the embedding server did not answer in time")]
    TimedOut,
    #[error("the embedding server is unavailable")]
    Unavailable(#[source] RemoteEmbeddingsError),
    #[error("the embedding server returned an unusable answer")]
    Failed(#[source] RemoteEmbeddingsError),
}

/// Sends one request at a time to the BGE server. The single semaphore permit
/// is held by the startup health check and then by each request, so recall
/// and idle work never compete on the server. A dropped request releases the
/// permit; the server may still finish it, which only delays the next one.
pub(crate) struct Embedder {
    client: RemoteEmbeddings,
    state: Arc<AtomicU8>,
    worker: Arc<Semaphore>,
}

impl Embedder {
    /// Returns immediately; a background health check settles the state.
    pub(crate) fn connect(client: RemoteEmbeddings) -> Self {
        let state = Arc::new(AtomicU8::new(EmbedderState::Connecting.as_u8()));
        let worker = Arc::new(Semaphore::new(1));
        let permit = Arc::clone(&worker)
            .try_acquire_owned()
            .expect("a new semaphore has its permit");
        let probe = client.clone();
        let probe_state = Arc::clone(&state);
        tokio::spawn(async move {
            let _permit = permit;
            let next = match probe.health_check().await {
                Ok(()) => EmbedderState::Ready,
                Err(_) => EmbedderState::Unavailable,
            };
            probe_state.store(next.as_u8(), Ordering::Release);
        });
        Self {
            client,
            state,
            worker,
        }
    }

    /// Identifies the model that produced stored vectors; a change re-embeds.
    pub(crate) fn fingerprint(&self) -> &'static str {
        MODEL_FINGERPRINT
    }

    pub(crate) fn state(&self) -> EmbedderState {
        EmbedderState::from_u8(self.state.load(Ordering::Acquire))
    }

    fn set_state(&self, state: EmbedderState) {
        self.state.store(state.as_u8(), Ordering::Release);
    }

    /// Embeds `text`, waiting up to `queue_wait` for the worker to be free.
    /// Any success marks the server ready again, so a restarted server heals
    /// recall without restarting Jarvis.
    pub(super) async fn embed(
        &self,
        text: &str,
        kind: EmbedKind,
        queue_wait: Duration,
    ) -> Result<Vec<f32>, EmbedError> {
        let _permit = tokio::time::timeout(queue_wait, self.worker.acquire())
            .await
            .map_err(|_| EmbedError::Busy)?
            .expect("the embedder semaphore is never closed");
        let result = match kind {
            EmbedKind::Query => self.client.embed_query(text).await,
            EmbedKind::Document => self.client.embed_document(text).await,
        };
        match result {
            Ok(vector) => {
                self.set_state(EmbedderState::Ready);
                Ok(vector)
            }
            Err(RemoteEmbeddingsError::Http(error)) if error.is_timeout() => {
                Err(EmbedError::TimedOut)
            }
            Err(error @ RemoteEmbeddingsError::Http(_)) => {
                self.set_state(EmbedderState::Unavailable);
                Err(EmbedError::Unavailable(error))
            }
            Err(error) => Err(EmbedError::Failed(error)),
        }
    }

    /// Resolves once no request or startup check holds the worker.
    pub(crate) async fn wait_until_idle(&self) {
        let _ = self.worker.acquire().await;
    }
}
