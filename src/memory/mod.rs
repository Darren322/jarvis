mod chunking;
mod extraction;
mod import;
mod index;
mod jobs;
mod recall;
mod types;

use std::{path::Path, path::PathBuf, sync::Arc, time::Duration};

use rig_core::providers::openai::CompletionModel;
use tokio::task::JoinHandle;

use crate::{
    clients::local_embeddings::{LocalEmbeddings, LocalEmbeddingsError, MODEL_FINGERPRINT},
    storage::{
        ForgetReceipt, JobId, MemoryId, MemoryJobLease, MemoryListFilter, MemoryRepository,
        MemorySummary, ProjectionLease, RetryDisposition, SourceId,
    },
};

pub(crate) use types::{
    ImportReceipt, JobRun, MemoryError, MemoryStatus, RecallSnapshot, RecallState,
};

const MODEL_INIT_TIMEOUT: Duration = Duration::from_secs(8);
const EMBEDDING_TIMEOUT: Duration = Duration::from_secs(8);
const RECALL_QUERY_TIMEOUT: Duration = Duration::from_secs(5);
const INDEX_OPEN_TIMEOUT: Duration = Duration::from_secs(5);
const INDEX_PAGE_SIZE: usize = 1;
const BACKFILL_PAGE_SIZE: usize = 16;
const EXTRACTION_TIMEOUT: Duration = Duration::from_secs(30);

// LEARNING: The facade keeps durable storage, model/index handles, and lifecycle
// leases together; the bounded state transitions themselves live in `jobs`.
pub(crate) struct MemoryService {
    repository: MemoryRepository,
    completion_model: CompletionModel,
    embeddings: Option<Arc<LocalEmbeddings>>,
    chunker: Option<chunking::TokenChunker>,
    index: Option<index::MemoryIndex>,
    index_dir: PathBuf,
    warning: Option<String>,
    chunking_warning: bool,
    active_model_load: Option<JoinHandle<Result<LocalEmbeddings, LocalEmbeddingsError>>>,
    active_embedding: Option<JoinHandle<Result<Vec<f32>, String>>>,
    active_job: Option<MemoryJobLease>,
    extraction_started: bool,
    active_projection: Option<ProjectionLease>,
}

impl MemoryService {
    /// Initialization reads local files only. A slow native model load remains
    /// owned by this service after the deadline; startup does not wait forever.
    pub(crate) async fn open(
        repository: MemoryRepository,
        completion_model: CompletionModel,
        model_dir: PathBuf,
        index_dir: PathBuf,
    ) -> Self {
        let embedding_path = model_dir.clone();
        let mut model_load =
            tokio::task::spawn_blocking(move || LocalEmbeddings::load_from_dir(&embedding_path));
        let (embeddings, active_model_load, mut warning) = match tokio::time::timeout(
            MODEL_INIT_TIMEOUT,
            &mut model_load,
        )
        .await
        {
            Ok(Ok(Ok(embeddings))) => (Some(Arc::new(embeddings)), None, None),
            Ok(Ok(Err(_))) | Ok(Err(_)) => (
                None,
                None,
                Some(
                    "Local embedding model unavailable; durable memory controls remain available."
                        .to_owned(),
                ),
            ),
            Err(_) => (
                None,
                Some(model_load),
                Some(
                    "Local embedding model is still loading; recall is temporarily unavailable."
                        .to_owned(),
                ),
            ),
        };

        let chunker = match chunking::TokenChunker::load(&model_dir) {
            Ok(chunker) => Some(chunker),
            Err(_) => {
                if warning.is_none() {
                    warning = Some(
                        "Pinned local tokenizer unavailable; imports remain stored but are not indexed."
                            .to_owned(),
                    );
                }
                None
            }
        };

        let (index, index_warning) = match tokio::time::timeout(
            INDEX_OPEN_TIMEOUT,
            index::MemoryIndex::open_rebuildable(&index_dir),
        )
        .await
        {
            Ok(Ok(index)) => (Some(index), None),
            Ok(Err(_)) | Err(_) => (
                None,
                Some(
                    "Local hybrid memory index unavailable or slow to open; SQLite-backed memory controls remain available."
                        .to_owned(),
                ),
            ),
        };
        if warning.is_none() {
            warning = index_warning;
        }

        if let Some(index) = index.as_ref() {
            let fingerprint_path = index_dir.join(".model-fingerprint");
            let previous = tokio::fs::read_to_string(&fingerprint_path).await.ok();
            if index.was_created || previous.as_deref() != Some(MODEL_FINGERPRINT) {
                match repository.schedule_projection_rebuild().await {
                    Ok(_) => {
                        if tokio::fs::write(&fingerprint_path, MODEL_FINGERPRINT)
                            .await
                            .is_err()
                            && warning.is_none()
                        {
                            warning = Some("Could not persist local model fingerprint.".to_owned());
                        }
                    }
                    Err(_) => {
                        if warning.is_none() {
                            warning = Some("Could not schedule local index rebuild.".to_owned());
                        }
                    }
                }
            }
        }

        Self {
            repository,
            completion_model,
            embeddings,
            chunker,
            index,
            index_dir,
            warning,
            chunking_warning: false,
            active_model_load,
            active_embedding: None,
            active_job: None,
            extraction_started: false,
            active_projection: None,
        }
    }

    pub(crate) async fn recall(&mut self, query: &str, excluded: &[SourceId]) -> RecallSnapshot {
        self.poll_model_load().await;
        if self.index.is_none() || self.embeddings.is_none() {
            return RecallSnapshot::unavailable(self.warning_or_default());
        }
        let embeddings = self.embeddings.as_ref().expect("checked above").clone();
        let vector = match self.embed(embeddings, query.to_owned(), true).await {
            Ok(vector) => vector,
            Err(_) => return RecallSnapshot::unavailable(self.warning_or_default()),
        };
        let Some(index) = self.index.as_ref() else {
            return RecallSnapshot::unavailable(self.warning_or_default());
        };
        match tokio::time::timeout(
            RECALL_QUERY_TIMEOUT,
            recall::recall(&self.repository, index, query, &vector, excluded),
        )
        .await
        {
            Ok(Ok(snapshot)) => snapshot,
            Ok(Err(_)) | Err(_) => RecallSnapshot::unavailable(self.warning_or_default()),
        }
    }

    pub(crate) async fn import_file(&mut self, path: &Path) -> Result<ImportReceipt, MemoryError> {
        import::import_file(&self.repository, path).await
    }

    pub(crate) async fn forget(&mut self, id: MemoryId) -> Result<ForgetReceipt, MemoryError> {
        let receipt = self.repository.forget(id).await?;
        let _ = self.process_projection_page(INDEX_PAGE_SIZE).await;
        Ok(receipt)
    }

    pub(crate) async fn forget_source(
        &mut self,
        id: SourceId,
    ) -> Result<ForgetReceipt, MemoryError> {
        let receipt = self.repository.forget_source(id).await?;
        let _ = self.process_projection_page(INDEX_PAGE_SIZE).await;
        Ok(receipt)
    }

    pub(crate) async fn list(
        &self,
        filter: MemoryListFilter,
    ) -> Result<Vec<MemorySummary>, MemoryError> {
        Ok(self.repository.list_memories(filter).await?)
    }

    pub(crate) async fn status(&mut self) -> Result<MemoryStatus, MemoryError> {
        // Status refreshes only workers that already completed. In particular, it
        // must not turn a quick console command into another wait on native inference.
        self.poll_model_load().await;
        self.reap_embedding().await;
        let stats = self.repository.stats().await?;
        Ok(MemoryStatus {
            backend_available: self.index.is_some() && self.embeddings.is_some(),
            warning: self.warning.clone(),
            stats,
        })
    }

    pub(crate) async fn retry(&self, id: JobId) -> Result<RetryDisposition, MemoryError> {
        Ok(self.repository.retry_failed(id).await?)
    }

    fn warning_or_default(&self) -> String {
        self.warning
            .clone()
            .unwrap_or_else(|| "Local memory recall is temporarily unavailable.".to_owned())
    }
}

#[cfg(test)]
#[path = "../../tests/unit/memory/lifecycle_tests.rs"]
mod lifecycle_tests;
