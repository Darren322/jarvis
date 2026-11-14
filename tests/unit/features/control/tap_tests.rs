use super::*;

#[test]
fn a_tap_means_one_thing_in_each_state() {
    let cases = [
        (AssistantState::Idle, TapAction::Listen),
        (AssistantState::Listening, TapAction::SendNow),
        (AssistantState::Transcribing, TapAction::Ignore),
        (AssistantState::Thinking, TapAction::DropAnswerThenListen),
        (AssistantState::Speaking, TapAction::StopSpeakingThenListen),
    ];
    for (state, expected) in cases {
        assert_eq!(tap_action(state), expected, "{state:?}");
    }
}
