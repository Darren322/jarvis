mod embedder;
mod import;
mod recall;
mod types;

use std::{path::Path, path::PathBuf, time::Duration};

use crate::storage::{
    ForgetReceipt, ImportReceipt, MemoryId, MemoryRepository, MemorySummary, SourceId,
};

pub(crate) use embedder::{EmbedError, EmbedderState};
pub(crate) use types::{IdlePass, MemoryError, MemoryStatus, RecallSnapshot};

use embedder::{EmbedKind, Embedder};

/// How long a foreground recall waits for an in-flight idle embedding.
const RECALL_QUEUE_WAIT: Duration = Duration::from_secs(2);
const IDLE_PAGE_SIZE: usize = 4;

/// Long-term memory: SQLite (via the repository) owns every durable fact, and
/// the embedder adds vectors in idle time. Recall is hybrid keyword + vector
/// search and degrades to keyword-only while vectors are unavailable.
pub(crate) struct MemoryService {
    repository: MemoryRepository,
    embedder: Embedder,
}

impl MemoryService {
    /// Returns immediately; the embedding model keeps loading in the background.
    pub(crate) fn open(repository: MemoryRepository, model_dir: PathBuf) -> Self {
        Self {
            repository,
            embedder: Embedder::load(model_dir),
        }
    }

    pub(crate) async fn recall(&self, query: &str, excluded: &[SourceId]) -> RecallSnapshot {
        let keyword_query = recall::keyword_query(query);
        let mut warning = None;
        let vector = match self
            .embedder
            .embed(query.to_owned(), EmbedKind::Query, RECALL_QUEUE_WAIT)
            .await
        {
            Ok(vector) => Some(vector),
            Err(error) => {
                warning = Some(format!("memory recall is keyword-only because {error}"));
                None
            }
        };
        if keyword_query.is_none() && vector.is_none() {
            return RecallSnapshot {
                warning,
                ..RecallSnapshot::default()
            };
        }

        match self.search(keyword_query, vector, excluded).await {
            Ok((documents, source_ids)) => RecallSnapshot {
                documents,
                source_ids,
                warning,
            },
            Err(error) => RecallSnapshot::unavailable(format!("memory recall failed: {error}")),
        }
    }

    async fn search(
        &self,
        keyword_query: Option<String>,
        vector: Option<Vec<f32>>,
        excluded: &[SourceId],
    ) -> Result<(Vec<rig_core::completion::Document>, Vec<SourceId>), MemoryError> {
        let mut candidates = recall::Candidates::default();
        if let Some(keyword_query) = keyword_query {
            candidates.keyword = self
                .repository
                .search_keyword(keyword_query, recall::SEARCH_LIMIT)
                .await?;
        }
        if let Some(vector) = vector {
            candidates.semantic = self
                .repository
                .search_vector(vector, self.embedder.fingerprint(), recall::SEARCH_LIMIT)
                .await?;
        }
        let ordered = candidates.fused();
        let hydrated = self.repository.hydrate(ordered.clone()).await?;
        Ok(recall::select_context(&ordered, hydrated, excluded))
    }

    pub(crate) async fn import_file(&self, path: &Path) -> Result<ImportReceipt, MemoryError> {
        import::import_file(&self.repository, path).await
    }

    pub(crate) async fn forget(&self, id: MemoryId) -> Result<ForgetReceipt, MemoryError> {
        Ok(self.repository.forget_memory(id).await?)
    }

    pub(crate) async fn forget_source(&self, id: SourceId) -> Result<ForgetReceipt, MemoryError> {
        Ok(self.repository.forget_source(id).await?)
    }

    pub(crate) async fn list(
        &self,
        query: Option<String>,
        limit: usize,
    ) -> Result<Vec<MemorySummary>, MemoryError> {
        Ok(self.repository.list_memories(query, limit).await?)
    }

    pub(crate) async fn status(&self) -> Result<MemoryStatus, MemoryError> {
        Ok(MemoryStatus {
            embedder: self.embedder.state(),
            stats: self.repository.stats(self.embedder.fingerprint()).await?,
        })
    }

    /// Discards every vector; idle passes then re-embed all memories.
    pub(crate) async fn rebuild(&self) -> Result<usize, MemoryError> {
        Ok(self.repository.reset_embeddings().await?)
    }

    /// Embeds a few pending memories. Safe to drop at any `.await`: each vector
    /// is stored in its own transaction, and nothing else is held durably.
    pub(crate) async fn run_idle_pass(&self) -> Result<IdlePass, MemoryError> {
        let mut pass = IdlePass::default();
        if self.embedder.state() != EmbedderState::Ready {
            return Ok(pass);
        }
        let model = self.embedder.fingerprint();
        let pending = self
            .repository
            .pending_embeddings(model, IDLE_PAGE_SIZE)
            .await?;
        for (id, text) in pending {
            let vector = self
                .embedder
                .embed(text, EmbedKind::Document, Duration::ZERO)
                .await?;
            if self.repository.store_embedding(id, model, vector).await? {
                pass.embedded += 1;
            }
        }
        Ok(pass)
    }

    /// Resolves once no native embedding task is running.
    pub(crate) async fn wait_until_embedder_idle(&self) {
        self.embedder.wait_until_idle().await;
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/memory/service_tests.rs"]
mod tests;
