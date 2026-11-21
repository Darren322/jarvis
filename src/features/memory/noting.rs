//! Noting facts while idle (fast-memory D7): the oldest completed turn that has not
//! been examined goes to the extraction run, and what comes back is checked and
//! stored as noted facts.
//!
//! The work is split where cancellation matters. `extract_next_turn` reads a turn and
//! asks the model: it holds and stores nothing, so the idle loop may drop it at any
//! `.await` when input arrives, and the turn is simply still unexamined. `commit`
//! then stores the facts and marks the turn examined in one transaction, and the
//! idle loop awaits it outside its race with input.

use std::{
    collections::HashMap,
    sync::{Mutex, PoisonError},
};

use crate::features::assistant::{AssistantError, FactExtractor};
use crate::storage::{Examination, FactsRepository, StorageError, TurnId};

/// Starts each fact in the extraction run's answer. Anything else in the answer
/// (including the `NONE` it gives for no facts) is not a fact and is ignored, so
/// malformed output yields nothing. The prompt in `note_facts.txt` asks for exactly
/// this.
const FACT_MARKER: &str = "FACT:";
/// The most facts one turn may yield; a run that lists more is rambling.
const MAX_FACTS_PER_TURN: usize = 5;
/// How many times the model run may fail on one turn before the pass stops trying
/// that turn until Jarvis restarts. A failure leaves the turn unexamined; this only
/// keeps a turn that always fails (a prompt that makes the model time out, say) from
/// costing a model run on every idle for ever.
const MAX_ATTEMPTS_PER_TURN: u8 = 3;

/// Why a noting pass did not finish. The messages are fixed: they are printed, and
/// must not carry what Darren said or what the model answered.
#[derive(Debug, thiserror::Error)]
pub(crate) enum NotingError {
    #[error("the model did not answer in time")]
    TimedOut,
    #[error("the model run failed")]
    Model(#[source] AssistantError),
    #[error(transparent)]
    Storage(#[from] StorageError),
}

impl From<AssistantError> for NotingError {
    fn from(error: AssistantError) -> Self {
        match error {
            AssistantError::RunTimeout => Self::TimedOut,
            other => Self::Model(other),
        }
    }
}

/// The facts a model run found in one turn, not yet stored. Untrusted: `commit`
/// holds each to the fact limits.
#[derive(Debug)]
pub(crate) struct ExtractedTurn {
    turn: TurnId,
    candidates: Vec<String>,
}

/// Examines turns for facts. Owns the extraction run and the facts list's repository.
pub(crate) struct FactNoter {
    extractor: FactExtractor,
    repository: FactsRepository,
    /// Failed model runs per turn since Jarvis started; never held across an `.await`.
    failures: Mutex<HashMap<TurnId, u8>>,
}

impl FactNoter {
    pub(crate) fn new(extractor: FactExtractor, repository: FactsRepository) -> Self {
        Self {
            extractor,
            repository,
            failures: Mutex::new(HashMap::new()),
        }
    }

    /// Reads the oldest turn still to examine and asks the model for its facts.
    /// `None` when no turn is waiting. Only Darren's words go to the model: the turn
    /// is read without Jarvis's answer.
    ///
    /// A failed or timed-out model run is an error, counted against the turn, and
    /// leaves the turn unexamined. Safe to drop at any `.await`: nothing is stored.
    pub(crate) async fn extract_next_turn(&self) -> Result<Option<ExtractedTurn>, NotingError> {
        let given_up = self.given_up();
        let Some(turn) = self.repository.next_unexamined_turn(&given_up).await? else {
            return Ok(None);
        };
        match self.extractor.extract(&turn.user_text).await {
            Ok(answer) => Ok(Some(ExtractedTurn {
                turn: turn.id,
                candidates: parse_candidates(&answer),
            })),
            Err(error) => {
                self.count_failure(turn.id);
                Err(error.into())
            }
        }
    }

    /// Stores the facts of `extracted` that pass the limits and are not already
    /// listed, and marks the turn examined, in one transaction (see
    /// `FactsRepository::finish_examination`).
    pub(crate) async fn commit(
        &self,
        extracted: ExtractedTurn,
    ) -> Result<Examination, NotingError> {
        let examination = self
            .repository
            .finish_examination(extracted.turn, extracted.candidates)
            .await?;
        self.failures().remove(&extracted.turn);
        Ok(examination)
    }

    fn failures(&self) -> std::sync::MutexGuard<'_, HashMap<TurnId, u8>> {
        // A poisoned lock still holds a usable count.
        self.failures.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn count_failure(&self, turn: TurnId) {
        let mut failures = self.failures();
        let count = failures.entry(turn).or_insert(0);
        *count = count.saturating_add(1);
    }

    fn given_up(&self) -> Vec<TurnId> {
        self.failures()
            .iter()
            .filter(|(_, count)| **count >= MAX_ATTEMPTS_PER_TURN)
            .map(|(turn, _)| *turn)
            .collect()
    }
}

/// The facts in a model answer: each line that starts with `FACT:`, without the
/// marker, trimmed, at most `MAX_FACTS_PER_TURN`. Everything else is dropped.
fn parse_candidates(answer: &str) -> Vec<String> {
    answer
        .lines()
        .filter_map(|line| line.trim_start().strip_prefix(FACT_MARKER))
        .map(str::trim)
        .filter(|fact| !fact.is_empty())
        .take(MAX_FACTS_PER_TURN)
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
#[path = "../../../tests/unit/features/memory/noting_tests.rs"]
mod tests;
