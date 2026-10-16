use std::path::PathBuf;

use crate::features::{
    conversation::ConversationSession,
    memory::{EmbedderState, MemoryService},
};
use crate::storage::{ForgetReceipt, ImportState, MemoryId, MemoryKind, MemorySummary, SourceId};

use super::{MemoryCommand, Notice};

const UNAVAILABLE: &str = "Local memory storage is unavailable.";

/// What "forget that" refers to. Only trusted foreground actions set it: the
/// last successful turn, an import, or a /memories listing with one result.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Focus {
    Source(SourceId),
    Memory(MemoryId),
    /// The last listing showed several memories, so "that" is ambiguous.
    AmbiguousDisplay,
}

/// Runs memory commands and remembers what the user is currently looking at.
#[derive(Debug, Default)]
pub(crate) struct MemoryControl {
    recent: Vec<MemorySummary>,
    focus: Option<Focus>,
}

impl MemoryControl {
    /// Forgets the current focus and listing, as after /reset.
    pub(crate) fn clear(&mut self) {
        self.recent.clear();
        self.focus = None;
    }

    /// Points "forget that" at the source saved for the latest successful turn.
    pub(crate) fn focus_on_turn(&mut self, source_id: Option<SourceId>) {
        self.focus = source_id.map(Focus::Source);
    }

    pub(crate) async fn run(
        &mut self,
        command: MemoryCommand,
        memory: Option<&MemoryService>,
        session: &mut ConversationSession,
    ) -> Vec<Notice> {
        let Some(memory) = memory else {
            return vec![Notice::Info(UNAVAILABLE.to_owned())];
        };
        match command {
            MemoryCommand::List(query) => self.list(memory, query).await,
            MemoryCommand::Forget(id) => {
                let result = memory.forget(id).await;
                self.after_forget(result, session)
            }
            MemoryCommand::ForgetFocus => self.forget_focus(memory, session).await,
            MemoryCommand::ForgetDetail(detail) => {
                self.forget_detail(memory, session, &detail).await
            }
            MemoryCommand::Import(path) => self.import(memory, session, &path).await,
            MemoryCommand::Status => status(memory).await,
            MemoryCommand::Rebuild => match memory.rebuild().await {
                Ok(reset) => vec![Notice::Info(format!(
                    "Queued {reset} memories for re-embedding while idle."
                ))],
                Err(error) => vec![Notice::Warning(format!(
                    "memory rebuild did not start: {error}"
                ))],
            },
        }
    }

    async fn list(&mut self, memory: &MemoryService, query: Option<String>) -> Vec<Notice> {
        self.recent = match memory.list(query, 20).await {
            Ok(records) => records,
            Err(error) => {
                self.recent.clear();
                self.focus = None;
                return vec![Notice::Warning(format!(
                    "memories could not be listed: {error}"
                ))];
            }
        };
        self.focus = match self.recent.as_slice() {
            [] => None,
            [only] => Some(Focus::Memory(only.id)),
            _ => Some(Focus::AmbiguousDisplay),
        };
        if self.recent.is_empty() {
            return vec![Notice::Info("No memories matched.".to_owned())];
        }
        self.recent
            .iter()
            .map(|record| {
                let fact = match record.kind {
                    MemoryKind::Fact => " [fact]",
                    MemoryKind::Excerpt => "",
                };
                Notice::Info(format!(
                    "{} [source {}]{fact} {}",
                    record.id.0, record.source_id.0, record.text
                ))
            })
            .collect()
    }

    async fn forget_focus(
        &mut self,
        memory: &MemoryService,
        session: &mut ConversationSession,
    ) -> Vec<Notice> {
        let result = match self.focus {
            Some(Focus::Source(source_id)) => memory.forget_source(source_id).await,
            Some(Focus::Memory(memory_id)) => memory.forget(memory_id).await,
            Some(Focus::AmbiguousDisplay) => {
                return vec![Notice::Info(
                    "The current /memories display has multiple records. Use /forget <id> to choose one; nothing was forgotten."
                        .to_owned(),
                )];
            }
            None => {
                return vec![Notice::Info(
                    "There is no current memory to forget. Use /memories to choose a record; nothing was forgotten."
                        .to_owned(),
                )];
            }
        };
        self.after_forget(result, session)
    }

    /// Forgets the single memory whose text contains `detail`, searching the
    /// last listing first and then the store. Ambiguity forgets nothing.
    async fn forget_detail(
        &mut self,
        memory: &MemoryService,
        session: &mut ConversationSession,
        detail: &str,
    ) -> Vec<Notice> {
        let needle = detail.to_lowercase();
        let mut candidates = self
            .recent
            .iter()
            .filter(|record| record.text.to_lowercase().contains(&needle))
            .map(|record| record.id)
            .collect::<Vec<_>>();
        match memory.list(Some(detail.to_owned()), 2).await {
            Ok(records) => candidates.extend(records.into_iter().map(|record| record.id)),
            Err(error) => {
                return vec![Notice::Warning(format!(
                    "forget detail could not be resolved: {error}"
                ))];
            }
        }
        candidates.sort();
        candidates.dedup();
        match candidates.as_slice() {
            [id] => {
                let result = memory.forget(*id).await;
                self.after_forget(result, session)
            }
            [] => vec![Notice::Info(
                "I could not find one memory matching that detail; nothing was forgotten."
                    .to_owned(),
            )],
            _ => vec![Notice::Info(
                "That detail matches multiple memories. Use /memories to choose an ID; nothing was forgotten."
                    .to_owned(),
            )],
        }
    }

    fn after_forget(
        &mut self,
        result: Result<ForgetReceipt, crate::features::memory::MemoryError>,
        session: &mut ConversationSession,
    ) -> Vec<Notice> {
        let receipt = match result {
            Ok(receipt) => receipt,
            Err(error) => {
                return vec![Notice::Warning(format!(
                    "the memory was not forgotten: {error}"
                ))];
            }
        };
        if receipt.is_empty() {
            return vec![Notice::Info(
                "No memory matched; nothing was forgotten.".to_owned(),
            )];
        }
        // Active history may quote what was just forgotten; drop it whole.
        session.clear_history();
        self.clear();
        vec![Notice::Info(format!(
            "Forgot {} memories from {} sources; {} archived turns were affected.",
            receipt.forgotten_memories, receipt.forgotten_sources, receipt.affected_turns,
        ))]
    }

    async fn import(
        &mut self,
        memory: &MemoryService,
        session: &mut ConversationSession,
        path: &str,
    ) -> Vec<Notice> {
        let receipt = match memory.import_file(&PathBuf::from(path)).await {
            Ok(receipt) => receipt,
            Err(error) => {
                return vec![Notice::Warning(format!("import was not saved: {error}"))];
            }
        };
        let state = match receipt.state {
            ImportState::Created => "created",
            ImportState::AlreadyCurrent => "already current",
            ImportState::Reactivated => "reactivated",
            ImportState::Forgotten => {
                return vec![Notice::Info(
                    "This file version was forgotten earlier and was not imported again."
                        .to_owned(),
                )];
            }
        };
        if receipt.state != ImportState::AlreadyCurrent {
            // A changed file can contradict facts in the active history.
            session.clear_history();
        }
        self.recent.clear();
        self.focus = Some(Focus::Source(receipt.source_id));
        vec![Notice::Info(format!(
            "Imported source {} ({} chunks, {state}).",
            receipt.source_id.0, receipt.chunks
        ))]
    }
}

async fn status(memory: &MemoryService) -> Vec<Notice> {
    match memory.status().await {
        Ok(status) => {
            let embedder = match status.embedder {
                EmbedderState::Connecting => "connecting",
                EmbedderState::Ready => "ready",
                EmbedderState::Unavailable => "unavailable (keyword-only recall)",
            };
            vec![Notice::Info(format!(
                "Memory: {} memories from {} conversations and {} imports; {} awaiting embedding; embedder {embedder}.",
                status.stats.memories,
                status.stats.conversation_sources,
                status.stats.import_sources,
                status.stats.pending_embeddings,
            ))]
        }
        Err(error) => vec![Notice::Warning(format!(
            "memory status is unavailable: {error}"
        ))],
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/features/control/memory_tests.rs"]
mod tests;
