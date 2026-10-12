use std::collections::VecDeque;

use rig_core::message::Message;

use crate::storage::SourceId;

const MAX_EXCHANGES: usize = 3;
const MAX_HISTORY_BYTES: usize = 16 * 1024;

const MISSING_MESSAGES_WARNING: &str =
    "Recent conversation context was cleared because Rig did not return message history.";
const OVERSIZED_BATCH_WARNING: &str = "Recent conversation context was cleared because the latest exchange exceeded the 16 KiB history limit.";
const HISTORY_SERIALIZATION_WARNING: &str =
    "Recent conversation context was cleared because its messages could not be serialized.";
// LEARNING: `VecDeque<HistoryBatch>` resembles Java's `ArrayDeque<HistoryBatch>`: each inner
// collection owns one complete exchange, while the outer queue can discard its oldest exchange.
// Rust's queue owns Message values, not Java-style shared object references.
#[derive(Default)]
pub(super) struct History {
    batches: VecDeque<HistoryBatch>,
}

// A whole native Rig message batch stays together with the trusted source
// dependencies that informed it, so forget can evict complete exchanges.
struct HistoryBatch {
    messages: Vec<Message>,
    source_ids: Vec<SourceId>,
}

impl History {
    pub(super) fn clear(&mut self) {
        self.batches.clear();
    }

    pub(super) fn messages(&self) -> Vec<Message> {
        // LEARNING: This resembles Java Streams `flatMap` and collection, but `.cloned().collect()`
        // builds an owned `Vec<Message>`.
        // Here `cloned()` copies each Message value rather than a Java-style shared object reference.
        self.batches
            .iter()
            .flat_map(|batch| batch.messages.iter().cloned())
            .collect()
    }

    pub(super) fn source_ids(&self) -> Vec<SourceId> {
        let mut sources = Vec::new();
        for source_id in self.batches.iter().flat_map(|batch| &batch.source_ids) {
            if !sources.contains(source_id) {
                sources.push(*source_id);
            }
        }
        sources
    }

    pub(super) fn record_success(
        &mut self,
        messages: Option<&[Message]>,
        recalled_source_ids: &[SourceId],
        current_source_id: Option<SourceId>,
    ) -> Option<&'static str> {
        // LEARNING: `Option<&[Message]>` is an optionally present borrowed slice; Rust makes absence
        // explicit instead of using null.
        // `let Some(..) else` handles absence early. The batch is copied only when the session will
        // own it.
        let Some(messages) = messages else {
            self.batches.clear();
            return Some(MISSING_MESSAGES_WARNING);
        };

        // LEARNING: `Result` carries a byte count or serialization error. This match guard checks the
        // cap only when serialization returned a byte count.
        match serialized_batch_bytes(messages) {
            Ok(bytes) if bytes <= MAX_HISTORY_BYTES => {}
            Ok(_) => {
                self.batches.clear();
                return Some(OVERSIZED_BATCH_WARNING);
            }
            Err(_) => {
                self.batches.clear();
                return Some(HISTORY_SERIALIZATION_WARNING);
            }
        }

        let mut source_ids = recalled_source_ids.to_vec();
        if let Some(source_id) = current_source_id
            && !source_ids.contains(&source_id)
        {
            source_ids.push(source_id);
        }
        self.batches.push_back(HistoryBatch {
            messages: messages.to_vec(),
            source_ids,
        });
        while self.batches.len() > MAX_EXCHANGES {
            self.batches.pop_front();
        }

        // LEARNING: `pop_front()` removes the oldest whole exchange. The loop trims batches until the
        // flattened history fits the byte cap.
        loop {
            match serialized_history_bytes(&self.batches) {
                Ok(bytes) if bytes <= MAX_HISTORY_BYTES => return None,
                Ok(_) => {
                    self.batches.pop_front();
                }
                Err(_) => {
                    self.batches.clear();
                    return Some(HISTORY_SERIALIZATION_WARNING);
                }
            }
        }
    }
}

pub(super) fn merge_source_dependencies(
    history_sources: &[SourceId],
    recalled_sources: &[SourceId],
) -> Vec<SourceId> {
    let mut dependencies = history_sources.to_vec();
    for source_id in recalled_sources {
        if !dependencies.contains(source_id) {
            dependencies.push(*source_id);
        }
    }
    dependencies
}

// LEARNING: Borrowed messages are serialized for an exact JSON byte count. The `Result` makes
// serialization failure explicit to the caller.
fn serialized_batch_bytes(messages: &[Message]) -> Result<usize, serde_json::Error> {
    serde_json::to_vec(messages).map(|serialized| serialized.len())
}

// LEARNING: `Vec<&Message>` is a temporary flattened view for counting.
// Sizing history does not clone the messages again.
fn serialized_history_bytes(batches: &VecDeque<HistoryBatch>) -> Result<usize, serde_json::Error> {
    let history: Vec<&Message> = batches
        .iter()
        .flat_map(|batch| batch.messages.iter())
        .collect();
    serde_json::to_vec(&history).map(|serialized| serialized.len())
}

#[cfg(test)]
#[path = "../../../tests/unit/features/conversation/history_tests.rs"]
mod tests;
