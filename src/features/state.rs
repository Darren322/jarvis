/// What the assistant is doing right now, for any front end to show.
///
/// Interface-neutral: the terminal prints it, and a later animation can draw it.
/// It is an enum so that a display can add its own dimension (colour, emotion)
/// without parsing text.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AssistantState {
    Idle,
    Listening,
    Transcribing,
    Thinking,
    Speaking,
}
