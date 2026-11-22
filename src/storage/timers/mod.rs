//! Timers: their own table (schema v7) and every SQL statement about it.
//!
//! Storage never reads the clock for a decision: each operation that depends on
//! "now" takes the instant (unix milliseconds) from its caller, so the timer service
//! can drive it with an injected clock. Every write is one immediate transaction, so
//! two callers racing for one timer (confirm against expiry) cannot both win.
#![expect(
    dead_code,
    reason = "the timer service (timers 03) is the first caller"
)]

use std::time::Duration;

use rusqlite::{TransactionBehavior, params};
use tokio_rusqlite::Connection;

use super::error::StorageError;

mod types;

pub(crate) use types::{Timer, TimerError, TimerId, TimerState};

const TIMER_COLUMNS: &str = "id, label, ends_at_unix_ms, created_at_unix_ms, state";

/// SQL for timers.
#[derive(Clone)]
pub(crate) struct TimersRepository {
    pub(super) connection: Connection,
}

impl TimersRepository {
    /// Adds a `Pending` timer unless `max_active` timers are already pending or
    /// running; the count and the insert are one transaction. Ended and cancelled
    /// timers do not count. The label is stored as given.
    pub(crate) async fn insert_pending(
        &self,
        label: Option<&str>,
        ends_at_unix_ms: i64,
        created_at_unix_ms: i64,
        max_active: usize,
    ) -> Result<Timer, TimerError> {
        let label = label.map(str::to_owned);
        self.connection
            .call(move |connection| {
                let transaction =
                    connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let active: i64 = transaction.query_row(
                    "SELECT COUNT(*) FROM timers WHERE state IN (?1, ?2)",
                    params![TimerState::Pending, TimerState::Running],
                    |row| row.get(0),
                )?;
                if usize::try_from(active).is_ok_and(|active| active >= max_active) {
                    // The transaction drops unfinished, which rolls it back.
                    return Ok(Err(TimerError::LimitReached { limit: max_active }));
                }
                transaction.execute(
                    "INSERT INTO timers (label, ends_at_unix_ms, created_at_unix_ms, state)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![
                        label,
                        ends_at_unix_ms,
                        created_at_unix_ms,
                        TimerState::Pending
                    ],
                )?;
                let timer = Timer {
                    id: TimerId(transaction.last_insert_rowid()),
                    label,
                    ends_at_unix_ms,
                    created_at_unix_ms,
                    state: TimerState::Pending,
                };
                transaction.commit()?;
                Ok(Ok(timer))
            })
            .await
            .map_err(StorageError::from)?
    }

    /// `Pending` to `Running`: Darren confirmed it.
    pub(crate) async fn confirm(&self, id: TimerId) -> Result<Timer, TimerError> {
        self.change_state(id, &[TimerState::Pending], TimerState::Running)
            .await
    }

    /// `Running` to `Ended`: its time came.
    pub(crate) async fn mark_ended(&self, id: TimerId) -> Result<Timer, TimerError> {
        self.change_state(id, &[TimerState::Running], TimerState::Ended)
            .await
    }

    /// `Pending`, `Running` or `Ended` to `Cancelled`. Cancelling an ended timer is
    /// how a tap removes it (D17).
    pub(crate) async fn cancel(&self, id: TimerId) -> Result<Timer, TimerError> {
        self.change_state(
            id,
            &[TimerState::Pending, TimerState::Running, TimerState::Ended],
            TimerState::Cancelled,
        )
        .await
    }

    /// Deletes every `Pending` timer created at or before `created_through_unix_ms`
    /// and returns how many. Their confirmation window is over: the service passes
    /// the instant one window ago to expire them, and the current instant at start
    /// to drop every leftover (D22). `Running` timers are never touched.
    pub(crate) async fn delete_pending_created_through(
        &self,
        created_through_unix_ms: i64,
    ) -> Result<usize, StorageError> {
        let deleted = self
            .connection
            .call(move |connection| {
                let transaction =
                    connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let deleted = transaction.execute(
                    "DELETE FROM timers WHERE state = ?1 AND created_at_unix_ms <= ?2",
                    params![TimerState::Pending, created_through_unix_ms],
                )?;
                transaction.commit()?;
                Ok(deleted)
            })
            .await?;
        Ok(deleted)
    }

    /// The timers to show at `now_unix_ms`, soonest end first (ties by id): every
    /// `Pending` and `Running` timer, and each `Ended` one that ended at most
    /// `ended_shown_for` before `now_unix_ms` (D19). `Cancelled` ones are never listed.
    pub(crate) async fn list_shown(
        &self,
        now_unix_ms: i64,
        ended_shown_for: Duration,
    ) -> Result<Vec<Timer>, StorageError> {
        let oldest_ended_shown = now_unix_ms.saturating_sub(millis(ended_shown_for));
        let timers = self
            .connection
            .call(move |connection| {
                let mut statement = connection.prepare(&format!(
                    "SELECT {TIMER_COLUMNS} FROM timers
                     WHERE state IN (?1, ?2) OR (state = ?3 AND ends_at_unix_ms >= ?4)
                     ORDER BY ends_at_unix_ms, id"
                ))?;
                statement
                    .query_map(
                        params![
                            TimerState::Pending,
                            TimerState::Running,
                            TimerState::Ended,
                            oldest_ended_shown
                        ],
                        timer_from_row,
                    )?
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .await?;
        Ok(timers)
    }

    /// One timer in any state, or `None` when no timer has that id.
    pub(crate) async fn get(&self, id: TimerId) -> Result<Option<Timer>, StorageError> {
        let timer = self
            .connection
            .call(move |connection| find(connection, id))
            .await?;
        Ok(timer)
    }

    /// `Running` timers whose end is at or before `now_unix_ms`, soonest first.
    pub(crate) async fn list_running_due(
        &self,
        now_unix_ms: i64,
    ) -> Result<Vec<Timer>, StorageError> {
        let timers = self
            .connection
            .call(move |connection| {
                let mut statement = connection.prepare(&format!(
                    "SELECT {TIMER_COLUMNS} FROM timers
                     WHERE state = ?1 AND ends_at_unix_ms <= ?2
                     ORDER BY ends_at_unix_ms, id"
                ))?;
                statement
                    .query_map(params![TimerState::Running, now_unix_ms], timer_from_row)?
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .await?;
        Ok(timers)
    }

    /// Moves one timer from any of `from` to `to` in one immediate transaction, or
    /// returns `NotInExpectedState` with what it found.
    async fn change_state(
        &self,
        id: TimerId,
        from: &'static [TimerState],
        to: TimerState,
    ) -> Result<Timer, TimerError> {
        self.connection
            .call(move |connection| {
                let transaction =
                    connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let Some(timer) = find(&transaction, id)? else {
                    return Ok(Err(TimerError::NotInExpectedState { id, found: None }));
                };
                if !from.contains(&timer.state) {
                    return Ok(Err(TimerError::NotInExpectedState {
                        id,
                        found: Some(timer.state),
                    }));
                }
                transaction.execute(
                    "UPDATE timers SET state = ?1 WHERE id = ?2",
                    params![to, id.0],
                )?;
                transaction.commit()?;
                Ok(Ok(Timer { state: to, ..timer }))
            })
            .await
            .map_err(StorageError::from)?
    }
}

/// A duration in whole milliseconds, saturating at `i64::MAX`.
fn millis(duration: Duration) -> i64 {
    i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
}

fn timer_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Timer> {
    Ok(Timer {
        id: TimerId(row.get(0)?),
        label: row.get(1)?,
        ends_at_unix_ms: row.get(2)?,
        created_at_unix_ms: row.get(3)?,
        state: row.get(4)?,
    })
}

/// Reads one timer. Takes `&Connection`, which a `&Transaction` derefs into.
fn find(connection: &rusqlite::Connection, id: TimerId) -> rusqlite::Result<Option<Timer>> {
    let mut statement =
        connection.prepare(&format!("SELECT {TIMER_COLUMNS} FROM timers WHERE id = ?1"))?;
    let mut rows = statement.query_map([id.0], timer_from_row)?;
    rows.next().transpose()
}

#[cfg(test)]
#[path = "../../../tests/unit/storage/timers/timers_tests.rs"]
mod tests;
