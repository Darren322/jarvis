//! What the idle noting pass (fast-memory 05) needs from SQLite: the oldest turn it has
//! not examined, and one transaction that adds the facts found in it and marks the
//! turn examined.
//!
//! The marker and the facts commit together, so a turn is examined exactly when its
//! facts are in the list: a pass cut off before the commit leaves both undone, and
//! one that reached the commit leaves both done. Nothing re-adds half a turn's facts.

use rusqlite::{TransactionBehavior, params, params_from_iter};

use super::error::StorageError;
use super::facts::{Fact, FactId, FactOrigin, FactsRepository, insert_within, list_all, validated};
use super::time::unix_time_ms;

/// The row id of an archived turn.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct TurnId(pub(crate) i64);

/// A completed turn the noting pass has not examined. It carries only what Darren
/// said: the answer Jarvis gave is not read here, so it cannot reach the model.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct UnexaminedTurn {
    pub(crate) id: TurnId,
    pub(crate) user_text: String,
}

/// What one turn's examination changed in the list.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct Examination {
    /// The facts added, in the order the model gave them.
    pub(crate) noted: Vec<Fact>,
    /// Older noted facts dropped to make room for them (fast-memory D8), oldest first.
    pub(crate) dropped: Vec<Fact>,
}

/// The oldest turn that is still to be examined. Keep the predicate in step with the
/// partial index `turns_awaiting_facts`. A turn qualifies only while its conversation
/// source is active: a turn Darren made Jarvis forget, or whose answer was built on
/// something forgotten, is never mined for facts again.
const OPEN_TURN: &str = "t.facts_examined = 0 AND t.outcome = 'completed'
     AND EXISTS (SELECT 1 FROM sources s WHERE s.turn_id = t.id AND s.state = 'active')";

impl FactsRepository {
    /// The oldest completed turn not yet examined, leaving out `skipped` (turns the
    /// caller has given up on for now). Dropped, failed and forgotten turns are never
    /// returned.
    pub(crate) async fn next_unexamined_turn(
        &self,
        skipped: &[TurnId],
    ) -> Result<Option<UnexaminedTurn>, StorageError> {
        let skipped: Vec<i64> = skipped.iter().map(|turn| turn.0).collect();
        let turn = self
            .connection
            .call(move |connection| {
                let placeholders = vec!["?"; skipped.len()].join(", ");
                let sql = format!(
                    "SELECT t.id, t.user_text FROM turns t
                     WHERE {OPEN_TURN} AND t.id NOT IN ({placeholders})
                     ORDER BY t.id LIMIT 1"
                );
                let mut statement = connection.prepare(&sql)?;
                let mut rows = statement.query_map(params_from_iter(skipped), |row| {
                    Ok(UnexaminedTurn {
                        id: TurnId(row.get(0)?),
                        user_text: row.get(1)?,
                    })
                })?;
                rows.next().transpose()
            })
            .await?;
        Ok(turn)
    }

    /// Adds the noted facts found in `turn` and marks it examined, in one immediate
    /// transaction. `candidates` are untrusted model output: each is trimmed and held
    /// to the fact limits, and one that is refused, or is the same as a fact already
    /// listed (case-insensitive after trimming), is skipped without an error.
    ///
    /// If the turn is no longer open (examined meanwhile, or forgotten while the model
    /// was running) nothing is added and nothing is marked.
    pub(crate) async fn finish_examination(
        &self,
        turn: TurnId,
        candidates: Vec<String>,
    ) -> Result<Examination, StorageError> {
        let created_at_unix_ms = unix_time_ms()?;
        let examination = self
            .connection
            .call(move |connection| {
                let transaction =
                    connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let still_open: bool = transaction.query_row(
                    &format!(
                        "SELECT EXISTS (SELECT 1 FROM turns t WHERE t.id = ?1 AND {OPEN_TURN})"
                    ),
                    [turn.0],
                    |row| row.get(0),
                )?;
                if !still_open {
                    return Ok(Examination::default());
                }

                let mut noted: Vec<Fact> = Vec::new();
                let mut dropped: Vec<Fact> = Vec::new();
                for candidate in &candidates {
                    let Ok(text) = validated(candidate) else {
                        continue;
                    };
                    let listed = list_all(&transaction)?;
                    if listed.iter().any(|fact| same_fact(&fact.text, text)) {
                        continue;
                    }
                    let Ok(added) =
                        insert_within(&transaction, text, FactOrigin::Noted, created_at_unix_ms)?
                    else {
                        continue;
                    };
                    noted.push(Fact {
                        id: added.id,
                        text: text.to_owned(),
                        origin: FactOrigin::Noted,
                        created_at_unix_ms,
                    });
                    dropped.extend(added.dropped);
                }
                // A fact this same pass added and then dropped for a later one was
                // never part of the list Darren sees: report neither side of it.
                let added_ids: Vec<FactId> = noted.iter().map(|fact| fact.id).collect();
                let dropped_ids: Vec<FactId> = dropped.iter().map(|fact| fact.id).collect();
                noted.retain(|fact| !dropped_ids.contains(&fact.id));
                dropped.retain(|fact| !added_ids.contains(&fact.id));

                transaction.execute(
                    "UPDATE turns SET facts_examined = 1 WHERE id = ?1",
                    params![turn.0],
                )?;
                transaction.commit()?;
                Ok(Examination { noted, dropped })
            })
            .await?;
        Ok(examination)
    }
}

/// Whether two fact texts are the same fact: equal after trimming, ignoring case.
fn same_fact(left: &str, right: &str) -> bool {
    left.trim().to_lowercase() == right.trim().to_lowercase()
}

#[cfg(test)]
#[path = "../../tests/unit/storage/noting_tests.rs"]
mod tests;
