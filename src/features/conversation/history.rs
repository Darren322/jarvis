use std::collections::VecDeque;

use rig_core::message::Message;

use crate::storage::SourceId;

/// The most exchanges the history holds. One more starts a trim.
const MAX_EXCHANGES: usize = 12;
/// The exchanges a trim leaves. Cutting back this far, instead of dropping one
/// exchange a turn, keeps the start of the conversation unchanged for the next
/// `MAX_EXCHANGES - TRIMMED_EXCHANGES` turns, so the model's prompt cache holds.
const TRIMMED_EXCHANGES: usize = 6;
/// The most serialized history the model's context has room for (see fast-memory
/// ticket 08 for the budget). One byte more starts a trim.
const MAX_HISTORY_BYTES: usize = 6 * 1024;
/// A trim also drops the oldest exchanges until the history is no larger than
/// this, so a run of long answers does not trim on every turn.
const TRIMMED_HISTORY_BYTES: usize = MAX_HISTORY_BYTES / 2;

const MISSING_MESSAGES_WARNING: &str =
    "Recent conversation context was cleared because Rig did not return message history.";
const OVERSIZED_BATCH_WARNING: &str = "Recent conversation context was cleared because the latest exchange exceeded the 6 KiB history limit.";
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

/// What recording a successful run did to the stored history.
#[derive(Debug, Eq, PartialEq)]
pub(super) enum Recorded {
    /// The exchange was added and nothing older was dropped.
    Kept,
    /// The exchange was added and the oldest exchanges were dropped, so the next
    /// request no longer starts the way the last one did.
    Trimmed,
    /// Nothing is stored any more; the text says why.
    Cleared(&'static str),
}

impl History {
    pub(super) fn clear(&mut self) {
        self.batches.clear();
    }

    /// Splits the stored messages before the last whole batch: `(earlier, latest)`.
    ///
    /// The boundary is a batch boundary, never a message count, because one
    /// exchange can hold a tool call and its result as well as the prompt and
    /// answer. With no stored batch both halves are empty; with one, `earlier`
    /// is empty.
    pub(super) fn split_last_exchange(&self) -> (Vec<Message>, Vec<Message>) {
        // LEARNING: This resembles Java Streams `flatMap` and collection, but `.cloned().collect()`
        // builds an owned `Vec<Message>`.
        // Here `cloned()` copies each Message value rather than a Java-style shared object reference.
        let earlier_count = self.batches.len().saturating_sub(1);
        let earlier = self
            .batches
            .iter()
            .take(earlier_count)
            .flat_map(|batch| batch.messages.iter().cloned())
            .collect();
        let latest = self
            .batches
            .back()
            .map(|batch| batch.messages.clone())
            .unwrap_or_default();
        (earlier, latest)
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
    ) -> Recorded {
        // LEARNING: `Option<&[Message]>` is an optionally present borrowed slice; Rust makes absence
        // explicit instead of using null.
        // `let Some(..) else` handles absence early. The batch is copied only when the session will
        // own it.
        let Some(messages) = messages else {
            self.batches.clear();
            return Recorded::Cleared(MISSING_MESSAGES_WARNING);
        };

        // LEARNING: `Result` carries a byte count or serialization error. This match guard checks the
        // cap only when serialization returned a byte count.
        match serialized_batch_bytes(messages) {
            Ok(bytes) if bytes <= MAX_HISTORY_BYTES => {}
            Ok(_) => {
                self.batches.clear();
                return Recorded::Cleared(OVERSIZED_BATCH_WARNING);
            }
            Err(_) => {
                self.batches.clear();
                return Recorded::Cleared(HISTORY_SERIALIZATION_WARNING);
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

        match self.trim_if_full() {
            Ok(recorded) => recorded,
            Err(_) => {
                self.batches.clear();
                Recorded::Cleared(HISTORY_SERIALIZATION_WARNING)
            }
        }
    }

    /// Cuts a full history back in one step: to `TRIMMED_EXCHANGES`, then further
    /// while it is still over `TRIMMED_HISTORY_BYTES`. Only whole exchanges go, and
    /// the newest one always stays (it already fits `MAX_HISTORY_BYTES`).
    fn trim_if_full(&mut self) -> Result<Recorded, serde_json::Error> {
        // LEARNING: `pop_front()` removes the oldest whole exchange, so a tool call is never
        // separated from its result.
        if self.batches.len() <= MAX_EXCHANGES
            && serialized_history_bytes(&self.batches)? <= MAX_HISTORY_BYTES
        {
            return Ok(Recorded::Kept);
        }
        while self.batches.len() > TRIMMED_EXCHANGES {
            self.batches.pop_front();
        }
        while self.batches.len() > 1
            && serialized_history_bytes(&self.batches)? > TRIMMED_HISTORY_BYTES
        {
            self.batches.pop_front();
        }
        Ok(Recorded::Trimmed)
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
