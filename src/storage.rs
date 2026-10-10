use std::{
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use rusqlite::{TransactionBehavior, params};
use tokio_rusqlite::Connection;

mod memory;
mod schema;
mod types;

pub(crate) use memory::MemoryRepository;
pub(crate) use types::*;

// LEARNING: Raw SQL stays in this storage seam. Services receive typed values
// and a cloned connection handle, while this module owns archive transactions.
#[derive(Clone)]
pub(crate) struct ConversationArchive {
    connection: Connection,
    session_id: i64,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum StorageError {
    #[error("archive I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("SQLite worker error: {0}")]
    Worker(#[from] tokio_rusqlite::Error<rusqlite::Error>),
    #[error("unsupported archive schema version: {0}")]
    UnsupportedSchemaVersion(i64),
    #[error("invalid memory source input: {0}")]
    InvalidInput(&'static str),
    #[error("memory source exceeds the {0}-byte limit")]
    SourceTooLarge(usize),
}

impl ConversationArchive {
    pub(crate) async fn open(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        let path = path.as_ref();
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            tokio::fs::create_dir_all(parent).await?;
        }

        let started_at_unix_ms = unix_time_ms()?;
        let connection = Connection::open(path).await?;
        let initialized = connection
            .call(move |connection| schema::initialize(connection, started_at_unix_ms))
            .await?;
        let session_id = match initialized {
            schema::OpenResult::Ready(session_id) => session_id,
            schema::OpenResult::Unsupported(version) => {
                return Err(StorageError::UnsupportedSchemaVersion(version));
            }
        };

        Ok(Self {
            connection,
            session_id,
        })
    }

    pub(crate) fn memory_repository(&self) -> MemoryRepository {
        MemoryRepository {
            connection: self.connection.clone(),
        }
    }

    /// Opens a fresh archived session on the same SQLite worker.
    pub(crate) async fn next_session(&self) -> Result<Self, StorageError> {
        let started_at_unix_ms = unix_time_ms()?;
        let session_id = self
            .connection
            .call(move |connection| {
                let transaction =
                    connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let session_id = schema::create_session(&transaction, started_at_unix_ms)?;
                transaction.commit()?;
                Ok(session_id)
            })
            .await?;
        Ok(Self {
            connection: self.connection.clone(),
            session_id,
        })
    }

    /// Archives a successful turn and links it to the bounded, SQLite-hydrated
    /// sources selected for that run in the same transaction.
    pub(crate) async fn append_turn_with_memory_and_sources(
        &self,
        user_text: &str,
        outcome: ArchiveOutcome,
        eligibility: MemoryEligibility,
        recalled_source_ids: &[SourceId],
    ) -> Result<ArchiveReceipt, StorageError> {
        let created_at_unix_ms = unix_time_ms()?;
        let session_id = self.session_id;
        // LEARNING: SQLite worker closures are `Send + 'static`, so borrowed
        // prompt/answer text becomes owned before the closure is queued.
        let user_text = user_text.to_owned();
        let (outcome_text, assistant_text) = match outcome {
            ArchiveOutcome::Completed(text) => ("completed", Some(text)),
            ArchiveOutcome::PromptFailed => ("prompt_failed", None),
            ArchiveOutcome::TimedOut => ("timed_out", None),
            ArchiveOutcome::InvalidResponse => ("invalid_response", None),
        };
        let memory_eligible =
            outcome_text == "completed" && eligibility == MemoryEligibility::Eligible;
        // Keep the message crossing the worker boundary bounded even when a
        // caller accidentally passes an oversized recall snapshot.
        let recalled_source_ids = recalled_source_ids
            .iter()
            .take(memory::MAX_RECALLED_SOURCES + 1)
            .copied()
            .collect::<Vec<_>>();
        let receipt = self
            .connection
            .call(move |connection| {
                let transaction =
                    connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                transaction.execute(
                    "INSERT INTO turns (
                        session_id, created_at_unix_ms, user_text, assistant_text, outcome,
                        memory_eligible
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        session_id,
                        created_at_unix_ms,
                        user_text,
                        assistant_text,
                        outcome_text,
                        memory_eligible,
                    ],
                )?;
                let turn_id = transaction.last_insert_rowid();
                let memory = if memory_eligible {
                    let assistant_text = assistant_text
                        .as_deref()
                        .expect("completed archive outcome has answer text");
                    let (source_id, state) = memory::insert_conversation_source(
                        &transaction,
                        turn_id,
                        &user_text,
                        assistant_text,
                        created_at_unix_ms,
                        MemoryEligibility::Eligible,
                        &recalled_source_ids,
                    )?;
                    transaction.commit()?;
                    return Ok(ArchiveReceipt {
                        turn_id,
                        source_id: Some(source_id),
                        memory: state,
                    });
                } else {
                    MemoryEnqueueState::NotEligible
                };
                transaction.commit()?;
                Ok(ArchiveReceipt {
                    turn_id,
                    source_id: None,
                    memory,
                })
            })
            .await?;
        Ok(receipt)
    }
}

// LEARNING: An enum carries only the data meaningful for its outcome. Failed
// runs remain safe archive rows with no assistant content or memory eligibility.
pub(crate) enum ArchiveOutcome {
    Completed(String),
    PromptFailed,
    TimedOut,
    InvalidResponse,
}

fn unix_time_ms() -> Result<i64, StorageError> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(std::io::Error::other)?;
    Ok(duration.as_millis() as i64)
}

#[cfg(test)]
#[path = "../tests/unit/storage/archive_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "../tests/unit/storage/memory/repository_tests.rs"]
mod memory_tests;
