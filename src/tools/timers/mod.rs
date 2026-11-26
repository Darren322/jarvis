//! The three timer tools (timers D12): `set_timer` proposes, `cancel_timer`
//! cancels, `list_timers` reads. They are separate tools because only
//! `set_timer` needs approval (D5), and their arguments stay flat because
//! Gemma's chat template cannot show `anyOf`.

pub(crate) mod cancel_timer;
pub(crate) mod list_timers;
pub(crate) mod resolve;
pub(crate) mod set_timer;

use orb_protocol::MAX_TIMER_LABEL_CHARS;
use rig_agent::tool::ToolExecutionError;
use thiserror::Error;

use crate::features::timers::TimerServiceError;
use crate::storage::TimerError;
use resolve::ResolveError;

/// Why a timer tool did nothing. Each message tells the model what to say or fix.
#[derive(Debug, Error)]
pub(crate) enum TimerToolError {
    #[error(
        "the label has {chars} characters but a timer label has at most {MAX_TIMER_LABEL_CHARS}: \
         call set_timer again with a shorter label"
    )]
    LabelTooLong { chars: usize },
    #[error(transparent)]
    Resolve(#[from] ResolveError),
    /// The 6th timer (D3). Nothing was created.
    #[error(
        "{limit} timers are already pending or running, the most there can be: nothing was \
         created; tell the user a new timer can be set once one ends or is cancelled"
    )]
    TooManyTimers { limit: usize },
    #[error("the timer service could not do it: tell the user the timers are unavailable")]
    Service(#[source] TimerServiceError),
}

impl TimerToolError {
    /// What the model is told: the message itself. Each one says what to tell the
    /// user, and Rig's default (`ToolExecutionError::from_error`) would replace it
    /// with a bare "failed", after which Gemma said a refused timer was set (seen
    /// on the Pi on 2026-11-26). No message carries a source's text.
    pub(crate) fn for_model(self) -> ToolExecutionError {
        let feedback = self.to_string();
        ToolExecutionError::from_error(self).with_model_feedback(feedback)
    }
}

impl From<TimerServiceError> for TimerToolError {
    fn from(error: TimerServiceError) -> Self {
        match error {
            TimerServiceError::Timer(TimerError::LimitReached { limit }) => {
                Self::TooManyTimers { limit }
            }
            other @ (TimerServiceError::Unavailable
            | TimerServiceError::WindowOver { .. }
            | TimerServiceError::NotPending { .. }
            | TimerServiceError::Timer(_)
            | TimerServiceError::Storage(_)) => Self::Service(other),
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/tools/timers/timers_tests.rs"]
mod tests;
