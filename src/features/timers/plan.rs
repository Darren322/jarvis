//! The timer service's decisions, as pure functions of the timers and one
//! "now" (unix milliseconds), so every boundary is tested with plain numbers.
//!
//! The service's loop only reads the clock, asks these functions, and applies
//! the answer to storage.

use std::time::Duration;

use crate::storage::{Timer, TimerState};

use super::{ENDED_TIMER_SHOWN_FOR, MISSED_TIMER_GRACE, TIMER_CONFIRM_WINDOW};

/// What one pass over the shown timers must do at `now`.
#[derive(Debug, Eq, PartialEq)]
pub(super) struct Pass {
    /// Pending timers whose confirmation window is over (D4), to delete.
    pub(super) expired: Vec<Timer>,
    /// Running timers whose end has come, to mark ended.
    pub(super) due: Vec<Timer>,
}

/// What to do with the shown timers at `now`.
pub(super) fn pass(shown: &[Timer], now_unix_ms: i64) -> Pass {
    let expired = shown
        .iter()
        .filter(|timer| timer.state == TimerState::Pending && is_window_over(timer, now_unix_ms))
        .cloned()
        .collect();
    let due = shown
        .iter()
        .filter(|timer| timer.state == TimerState::Running && timer.ends_at_unix_ms <= now_unix_ms)
        .cloned()
        .collect();
    Pass { expired, due }
}

/// Whether `timer`'s confirmation window is over at `now`. Confirming at
/// exactly [`TIMER_CONFIRM_WINDOW`] after it was proposed still sets it; one
/// millisecond later does not.
pub(super) fn is_window_over(timer: &Timer, now_unix_ms: i64) -> bool {
    now_unix_ms.saturating_sub(timer.created_at_unix_ms) > millis(TIMER_CONFIRM_WINDOW)
}

/// The latest creation time of a pending timer whose window is over at `now`,
/// for `TimersRepository::delete_pending_created_through` (inclusive).
pub(super) fn expired_created_through(now_unix_ms: i64) -> i64 {
    now_unix_ms
        .saturating_sub(millis(TIMER_CONFIRM_WINDOW))
        .saturating_sub(1)
}

/// When the shown timers next need a pass: the soonest of a running timer's
/// end, the first millisecond after a pending timer's window, and the first
/// millisecond after an ended row's [`ENDED_TIMER_SHOWN_FOR`]. `None` when
/// nothing is shown.
pub(super) fn next_wake(shown: &[Timer]) -> Option<i64> {
    shown
        .iter()
        .map(|timer| match timer.state {
            TimerState::Running => timer.ends_at_unix_ms,
            TimerState::Pending => timer
                .created_at_unix_ms
                .saturating_add(millis(TIMER_CONFIRM_WINDOW))
                .saturating_add(1),
            TimerState::Ended => timer
                .ends_at_unix_ms
                .saturating_add(millis(ENDED_TIMER_SHOWN_FOR))
                .saturating_add(1),
            // Never listed; a cancelled timer needs no pass.
            TimerState::Cancelled => i64::MAX,
        })
        .min()
}

/// A running timer that was due when Jarvis started (D9).
#[derive(Debug, Eq, PartialEq)]
pub(super) enum Overdue {
    /// Ended less than [`MISSED_TIMER_GRACE`] ago: it fires now, saying how late.
    Late(Duration),
    /// Ended [`MISSED_TIMER_GRACE`] ago or more: it is dropped with a notice.
    Missed,
}

/// How a running timer found due at start is handled (D9). One that ends
/// exactly at start is not late at all.
pub(super) fn overdue(timer: &Timer, now_unix_ms: i64) -> Overdue {
    let late_ms = now_unix_ms.saturating_sub(timer.ends_at_unix_ms).max(0);
    if late_ms < millis(MISSED_TIMER_GRACE) {
        // `late_ms` is non-negative here, so the conversion cannot fail.
        Overdue::Late(Duration::from_millis(u64::try_from(late_ms).unwrap_or(0)))
    } else {
        Overdue::Missed
    }
}

/// A duration in whole milliseconds, saturating at `i64::MAX`.
pub(super) fn millis(duration: Duration) -> i64 {
    i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
}

#[cfg(test)]
#[path = "../../../tests/unit/features/timers/plan_tests.rs"]
mod tests;
