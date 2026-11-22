//! The timers on the screen (timers 07): the list going out and the taps coming
//! back.
//!
//! The wire speaks `u64` ids and times and the screen's own `Timer` type; storage
//! speaks `TimerId(i64)` and `i64` milliseconds. Converting between them is the
//! orb interface's job and never panics: a timer that does not fit is left out of
//! the list, and a tap whose id does not fit is ignored like an unknown id.

use orb_protocol::{FromOrb, ToOrb};
use tokio::sync::{mpsc, watch};

use crate::features::control::TimerTap;
use crate::storage::{Timer, TimerId, TimerState};

/// The whole list as the screen is sent it. A timer that cannot be written on the
/// wire (an id or end time that is negative, or a label over the limit) is left
/// out: the rest still show.
pub(super) fn timers_message(timers: &[Timer]) -> ToOrb {
    ToOrb::Timers {
        timers: timers.iter().filter_map(wire_timer).collect(),
    }
}

fn wire_timer(timer: &Timer) -> Option<orb_protocol::Timer> {
    let state = match timer.state {
        TimerState::Pending => orb_protocol::TimerState::Pending,
        TimerState::Running => orb_protocol::TimerState::Running,
        TimerState::Ended => orb_protocol::TimerState::Ended,
        // A cancelled timer is never listed; were one to appear it is not shown.
        TimerState::Cancelled => return None,
    };
    orb_protocol::Timer::new(
        u64::try_from(timer.id.0).ok()?,
        timer.label.clone(),
        u64::try_from(timer.ends_at_unix_ms).ok()?,
        state,
    )
    .ok()
}

/// Sends the list to the orbs once, and again on every change of `shown`, until
/// the timers are gone or the orb server is. Intermediate values a slow orb never
/// needs are skipped by the `watch`; the last one is never lost.
///
/// Safe to drop at any `.await`: it holds nothing, and the next change of the
/// list is sent by whoever runs this next.
pub(crate) async fn publish_timers(
    mut shown: watch::Receiver<Vec<Timer>>,
    to_orb: mpsc::Sender<ToOrb>,
) {
    loop {
        let message = timers_message(&shown.borrow_and_update());
        if to_orb.send(message).await.is_err() || shown.changed().await.is_err() {
            return;
        }
    }
}

/// What the orbs said about timers, as the answers Jarvis acts on.
pub(crate) struct TimerTaps {
    messages: mpsc::Receiver<FromOrb>,
}

impl TimerTaps {
    pub(super) fn new(messages: mpsc::Receiver<FromOrb>) -> Self {
        Self { messages }
    }

    /// The next tap on a timer, in the order they came. `None` once the server is
    /// gone. A message whose id cannot be a stored id is skipped.
    ///
    /// Safe to drop at any `.await`: a message is only taken when this returns.
    pub(crate) async fn next(&mut self) -> Option<TimerTap> {
        loop {
            let message = self.messages.recv().await?;
            if let Some(tap) = timer_tap(message) {
                return Some(tap);
            }
        }
    }
}

fn timer_tap(message: FromOrb) -> Option<TimerTap> {
    match message {
        FromOrb::ConfirmTimer { id } => stored_id(id).map(TimerTap::Confirm),
        FromOrb::RejectTimer { id } => stored_id(id).map(TimerTap::Reject),
        FromOrb::CancelTimer { id } => stored_id(id).map(TimerTap::Cancel),
        // Never routed here; nothing to answer if one were.
        FromOrb::Tap {} | FromOrb::PanelClosed {} => None,
    }
}

fn stored_id(id: u64) -> Option<TimerId> {
    i64::try_from(id).ok().map(TimerId)
}

#[cfg(test)]
#[path = "../../../tests/unit/interfaces/orb/timers_tests.rs"]
mod tests;
