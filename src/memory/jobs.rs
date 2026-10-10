use std::sync::Arc;

use tokio::time::timeout;

use crate::storage::{CommitDisposition, ProjectionOperation, ProjectionOutcome, SafeFailure};

use super::{
    BACKFILL_PAGE_SIZE, EMBEDDING_TIMEOUT, EXTRACTION_TIMEOUT, INDEX_OPEN_TIMEOUT, INDEX_PAGE_SIZE,
    MemoryError, MemoryService, index, types::JobRun,
};

const INDEX_APPLY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

impl MemoryService {
    pub(super) async fn poll_model_load(&mut self) {
        let finished = self
            .active_model_load
            .as_ref()
            .is_some_and(tokio::task::JoinHandle::is_finished);
        if !finished {
            return;
        }

        let Some(handle) = self.active_model_load.take() else {
            return;
        };
        self.settle_model_load(handle.await);
    }

    /// Waits for one retained native worker without consuming its handle while
    /// pending. If the caller drops this future on input, the service still owns
    /// the JoinHandle and can reap/settle it later; Tokio cannot abort started
    /// blocking inference safely.
    pub(crate) async fn wait_for_native_worker(&mut self) {
        enum NativeWorkerResult {
            Model(
                Box<
                    Result<
                        Result<
                            crate::clients::local_embeddings::LocalEmbeddings,
                            crate::clients::local_embeddings::LocalEmbeddingsError,
                        >,
                        tokio::task::JoinError,
                    >,
                >,
            ),
            Embedding(Result<Result<Vec<f32>, String>, tokio::task::JoinError>),
        }

        let result = {
            match (
                self.active_model_load.as_mut(),
                self.active_embedding.as_mut(),
            ) {
                (Some(model), Some(embedding)) => tokio::select! {
                    result = &mut *model => NativeWorkerResult::Model(Box::new(result)),
                    result = &mut *embedding => NativeWorkerResult::Embedding(result),
                },
                (Some(model), None) => NativeWorkerResult::Model(Box::new(model.await)),
                (None, Some(embedding)) => NativeWorkerResult::Embedding(embedding.await),
                (None, None) => return,
            }
        };

        match result {
            NativeWorkerResult::Model(result) => {
                self.active_model_load = None;
                self.settle_model_load(*result);
            }
            NativeWorkerResult::Embedding(_result) => {
                self.active_embedding = None;
            }
        }
    }

    fn settle_model_load(
        &mut self,
        result: Result<
            Result<
                crate::clients::local_embeddings::LocalEmbeddings,
                crate::clients::local_embeddings::LocalEmbeddingsError,
            >,
            tokio::task::JoinError,
        >,
    ) {
        match result {
            Ok(Ok(embeddings)) => {
                self.embeddings = Some(Arc::new(embeddings));
                if self
                    .warning
                    .as_deref()
                    .is_some_and(|warning| warning.contains("still loading"))
                {
                    self.refresh_ready_warning();
                }
            }
            Ok(Err(_)) | Err(_) => {
                self.warning = Some(
                    "Local embedding model unavailable; durable memory controls remain available."
                        .to_owned(),
                );
            }
        }
    }

    pub(super) async fn embed(
        &mut self,
        embeddings: Arc<crate::clients::local_embeddings::LocalEmbeddings>,
        text: String,
        query: bool,
    ) -> Result<Vec<f32>, MemoryError> {
        self.poll_model_load().await;
        self.reap_embedding().await;
        if self.active_model_load.is_some() || self.active_embedding.is_some() {
            return Err(MemoryError::WorkerBusy);
        }

        self.active_embedding = Some(tokio::task::spawn_blocking(move || {
            let result = if query {
                embeddings.embed_query(&text)
            } else {
                embeddings.embed_document(&text)
            };
            result.map_err(|error| error.to_string())
        }));

        let result = {
            let handle = self
                .active_embedding
                .as_mut()
                .expect("worker was just installed");
            timeout(EMBEDDING_TIMEOUT, handle).await
        };
        match result {
            Err(_) => Err(MemoryError::WorkerBusy),
            Ok(joined) => {
                self.active_embedding = None;
                match joined {
                    Ok(Ok(vector)) => Ok(vector),
                    Ok(Err(message)) => Err(MemoryError::Worker(message)),
                    Err(error) => Err(MemoryError::Worker(format!(
                        "local embedding task failed: {error}"
                    ))),
                }
            }
        }
    }

    /// One cooperative idle pass handles bounded backfill, one part, one index
    /// projection, and one extraction. Doing each lane once prevents a steady
    /// stream of new sources from starving extraction or historical projection.
    pub(crate) async fn run_one_idle_job(&mut self) -> Result<JobRun, MemoryError> {
        self.settle_cancelled_work().await?;
        if self.native_worker_busy().await {
            return Err(MemoryError::WorkerBusy);
        }

        let mut run = JobRun::default();

        let backfill = self
            .repository
            .backfill_projection_page(BACKFILL_PAGE_SIZE)
            .await?;
        run.backfill_turns_examined = backfill.examined_turns;

        if let Some(chunker) = self.chunker.as_ref() {
            let pending = self
                .repository
                .pending_source_parts(1)
                .await?
                .into_iter()
                .next();
            if let Some(pending) = pending {
                let chunks = match chunker.chunk(&pending.part) {
                    Ok(chunks) => chunks,
                    Err(_) => {
                        // LEARNING: A deterministic tokenizer failure must not pin the
                        // oldest part at the queue head forever. Mark it complete without
                        // projection so the stored source remains canonical and other
                        // extraction/projection work can proceed.
                        self.chunking_warning = true;
                        if self.warning.is_none() {
                            self.warning = Some(
                                "One local source part could not be tokenized and was left out of recall."
                                    .to_owned(),
                            );
                        }
                        Vec::new()
                    }
                };
                let disposition = self
                    .repository
                    .commit_source_chunks(
                        pending.source.id,
                        pending.source.revision_sha256,
                        pending.part.index,
                        chunks,
                        true,
                    )
                    .await?;
                if disposition == CommitDisposition::Committed {
                    run.source_parts_processed = 1;
                }
            }
        }

        run.projection_updates = self.process_projection_page(INDEX_PAGE_SIZE).await?;

        let Some(lease) = self.repository.claim_next_job().await? else {
            return Ok(run);
        };
        run.job_id = Some(lease.id);
        run.attempt = Some(lease.attempt);
        self.active_job = Some(lease.clone());
        self.extraction_started = false;

        let source = match self.repository.get_source(lease.source_id).await {
            Ok(Some(source)) => source,
            Ok(None) => {
                self.release_active_job().await?;
                return Ok(run);
            }
            Err(error) => {
                self.release_active_job().await?;
                return Err(error.into());
            }
        };

        let payload = match super::extraction::prepare_user_source(&source) {
            Ok(payload) => payload,
            Err(error) => {
                self.fail_active_job(extraction_failure(&error)).await?;
                return Ok(run);
            }
        };
        let Some(payload) = payload else {
            let disposition = self
                .repository
                .commit_extraction(lease.clone(), Vec::new())
                .await?;
            self.active_job = None;
            if disposition != CommitDisposition::LeaseLost {
                run.committed_records = 0;
            }
            return Ok(run);
        };

        // From here the model request is admitted. Dropping this async future may
        // stop waiting without proving a remote request had no effect; preserve
        // the durable attempt, while keeping reported provider calls accurate.
        self.extraction_started = true;
        run.model_calls = 1;
        let extracted = timeout(
            EXTRACTION_TIMEOUT,
            super::extraction::extract_user_memories(&self.completion_model, &source, payload),
        )
        .await;
        let (records, usage) = match extracted {
            Ok(Ok(result)) => result,
            Ok(Err(error)) => {
                self.fail_active_job(extraction_failure(&error)).await?;
                return Ok(run);
            }
            Err(_) => {
                self.fail_active_job(SafeFailure::ExtractionUnavailable)
                    .await?;
                return Ok(run);
            }
        };

        // LEARNING: Rig represents missing provider usage as all-zero metrics. Keep
        // that distinct from an observed usage report instead of presenting zero as fact.
        run.usage = (usage.total_tokens > 0).then_some(usage);
        let record_count = records.len();
        let disposition = self.repository.commit_extraction(lease, records).await?;
        self.active_job = None;
        self.extraction_started = false;
        if matches!(disposition, CommitDisposition::Committed) {
            run.committed_records = record_count;
        }
        Ok(run)
    }

    /// Settles durable leases immediately, but never waits on an unfinished
    /// `spawn_blocking` inference. The JoinHandle remains owned and blocks later
    /// native work until it completes; Tokio cannot hard-abort started blocking work.
    pub(crate) async fn interrupt_idle_work(&mut self) -> Result<(), MemoryError> {
        self.poll_model_load().await;
        self.reap_embedding().await;
        let worker_busy = self.active_model_load.is_some() || self.active_embedding.is_some();

        if let Some(lease) = self.active_projection.as_ref().cloned() {
            self.repository
                .finish_projection(lease, ProjectionOutcome::Failed)
                .await?;
            self.active_projection = None;
        }
        self.release_active_job().await?;

        if worker_busy {
            Err(MemoryError::WorkerBusy)
        } else {
            Ok(())
        }
    }

    /// Rebuild only schedules canonical SQLite projections; idle work consumes
    /// them one record at a time so a rebuild cannot monopolize foreground input.
    pub(crate) async fn rebuild(&mut self) -> Result<usize, MemoryError> {
        self.settle_cancelled_work().await?;
        if self.index.is_none() {
            let opened = timeout(
                INDEX_OPEN_TIMEOUT,
                index::MemoryIndex::open_rebuildable(&self.index_dir),
            )
            .await
            .map_err(|_| {
                MemoryError::Backend("local hybrid memory index open timed out".to_owned())
            })?
            .map_err(|error| MemoryError::Backend(error.to_string()))?;
            self.index = Some(opened);
        }
        if self.embeddings.is_none() {
            return Err(MemoryError::Backend(self.warning_or_default()));
        }
        self.refresh_ready_warning();
        let changed = self.repository.schedule_projection_rebuild().await?;
        let fingerprint_path = self.index_dir.join(".model-fingerprint");
        tokio::fs::write(
            fingerprint_path,
            crate::clients::local_embeddings::MODEL_FINGERPRINT,
        )
        .await?;
        Ok(changed)
    }

    pub(super) async fn process_projection_page(
        &mut self,
        limit: usize,
    ) -> Result<usize, MemoryError> {
        if self.index.is_none() || self.embeddings.is_none() {
            return Ok(0);
        }
        let page = self.repository.claim_projection_page(limit).await?;
        let mut applied = 0;
        for projection in page.items {
            self.active_projection = Some(projection.clone());
            let vector = if projection.operation == ProjectionOperation::Upsert {
                let embeddings = self.embeddings.as_ref().expect("checked above").clone();
                self.embed(embeddings, projection.text.clone(), false)
                    .await
                    .ok()
            } else {
                None
            };
            let can_apply = projection.operation == ProjectionOperation::Delete || vector.is_some();
            let outcome = if can_apply {
                let apply = self
                    .index
                    .as_mut()
                    .expect("index presence checked before projection page")
                    .apply(&projection, vector);
                match timeout(INDEX_APPLY_TIMEOUT, apply).await {
                    Ok(Ok(())) => ProjectionOutcome::Applied,
                    Ok(Err(_)) | Err(_) => ProjectionOutcome::Failed,
                }
            } else {
                ProjectionOutcome::Failed
            };
            let finished = self
                .repository
                .finish_projection(projection, outcome)
                .await?;
            if finished && outcome == ProjectionOutcome::Applied {
                applied += 1;
            }
            self.active_projection = None;
        }
        Ok(applied)
    }

    async fn fail_active_job(&mut self, failure: SafeFailure) -> Result<(), MemoryError> {
        if let Some(lease) = self.active_job.as_ref().cloned() {
            self.repository.fail_job(lease, failure).await?;
            self.active_job = None;
        }
        self.extraction_started = false;
        Ok(())
    }

    async fn release_active_job(&mut self) -> Result<(), MemoryError> {
        if let Some(lease) = self.active_job.as_ref().cloned() {
            self.repository
                .release_job(lease, self.extraction_started)
                .await?;
            self.active_job = None;
        }
        self.extraction_started = false;
        Ok(())
    }

    async fn settle_cancelled_work(&mut self) -> Result<(), MemoryError> {
        self.poll_model_load().await;
        self.reap_embedding().await;
        if self.active_job.is_some() || self.active_projection.is_some() {
            self.interrupt_idle_work().await?;
        }
        if self.active_model_load.is_some() || self.active_embedding.is_some() {
            return Err(MemoryError::WorkerBusy);
        }
        Ok(())
    }

    async fn native_worker_busy(&mut self) -> bool {
        self.poll_model_load().await;
        self.reap_embedding().await;
        self.active_model_load.is_some() || self.active_embedding.is_some()
    }

    pub(super) async fn reap_embedding(&mut self) {
        if self
            .active_embedding
            .as_ref()
            .is_some_and(tokio::task::JoinHandle::is_finished)
            && let Some(handle) = self.active_embedding.take()
        {
            let _ = handle.await;
        }
    }

    fn refresh_ready_warning(&mut self) {
        self.warning = if self.index.is_none() {
            Some(
                "Local hybrid memory index unavailable; SQLite-backed memory controls remain available."
                    .to_owned(),
            )
        } else if self.chunker.is_none() {
            Some(
                "Pinned local tokenizer unavailable; imports remain stored but are not indexed."
                    .to_owned(),
            )
        } else if self.chunking_warning {
            Some(
                "One or more local source parts could not be tokenized and were left out of recall."
                    .to_owned(),
            )
        } else {
            None
        };
    }
}

fn extraction_failure(error: &super::extraction::ExtractionError) -> SafeFailure {
    match error {
        super::extraction::ExtractionError::SourceTooLarge => SafeFailure::SourceTooLarge,
        super::extraction::ExtractionError::Invalid => SafeFailure::InvalidExtraction,
        super::extraction::ExtractionError::Model(_) => SafeFailure::ExtractionUnavailable,
    }
}
