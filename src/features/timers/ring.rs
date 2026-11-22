//! The sound when a timer ends (timers D6), played the moment the service marks
//! it ended, whatever Jarvis is doing; the spoken reminder waits for idle.

use std::{collections::HashSet, convert::Infallible};

use tokio::sync::watch;

use crate::storage::{Timer, TimerId, TimerState};

/// The timers in `shown` that are ended and were not ended in `seen`.
pub(super) fn newly_ended(seen: &HashSet<TimerId>, shown: &[Timer]) -> Vec<TimerId> {
    shown
        .iter()
        .filter(|timer| timer.state == TimerState::Ended && !seen.contains(&timer.id))
        .map(|timer| timer.id)
        .collect()
}

/// Calls `ring` once for every timer that becomes ended, for as long as it is
/// polled. Timers already ended when it starts do not ring. Never returns while
/// the service runs; safe to drop at any `.await`.
pub(crate) async fn ring_when_timers_end(
    mut shown: watch::Receiver<Vec<Timer>>,
    ring: impl Fn(),
) -> Infallible {
    let mut seen: HashSet<TimerId> = shown
        .borrow_and_update()
        .iter()
        .filter(|timer| timer.state == TimerState::Ended)
        .map(|timer| timer.id)
        .collect();
    loop {
        if shown.changed().await.is_err() {
            // The service is gone: nothing will end any more.
            return std::future::pending().await;
        }
        let ended = newly_ended(&seen, &shown.borrow_and_update());
        for id in ended {
            seen.insert(id);
            ring();
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/features/timers/ring_tests.rs"]
mod tests;
