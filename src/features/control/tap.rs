//! What a tap on the orb means.
//!
//! A tap carries nothing, so Jarvis decides from the state it is really in when it
//! handles the tap (visual-jarvis spec D7). The orb never decides: it sees states
//! late and would race. Each interface loop calls [`tap_action`] with its own
//! state and carries the action out.

use crate::features::state::AssistantState;

/// One tap from a touch front end. Untrusted input: it grants nothing beyond the
/// action [`tap_action`] maps it to.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Tap;

/// What a tap does in one state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TapAction {
    /// Start listening; the recording ends on silence or on the next tap.
    Listen,
    /// Stop the recording now and send what was heard.
    SendNow,
    /// Nothing: the words are already on their way.
    Ignore,
    /// Drop the answer being written, without archiving it, then listen.
    DropAnswerThenListen,
    /// Stop the voice, then listen. An answer that is finished stays as it is; one
    /// still being written is dropped with the voice.
    StopSpeakingThenListen,
}

pub(crate) fn tap_action(state: AssistantState) -> TapAction {
    match state {
        AssistantState::Idle => TapAction::Listen,
        AssistantState::Listening => TapAction::SendNow,
        AssistantState::Transcribing => TapAction::Ignore,
        AssistantState::Thinking => TapAction::DropAnswerThenListen,
        AssistantState::Speaking => TapAction::StopSpeakingThenListen,
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/features/control/tap_tests.rs"]
mod tests;
