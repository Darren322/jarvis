use std::time::Duration;

use crate::features::assistant::{CallUsage, ModelStage};

use super::{error_chain, format_first_audio, format_model_stage};

fn stage(first_text: Option<Duration>) -> ModelStage {
    ModelStage {
        turn: 1,
        completed: true,
        elapsed: Some(Duration::from_millis(900)),
        first_text,
        usage: CallUsage::Unavailable,
    }
}

#[test]
fn model_stage_line_reports_time_to_first_text_when_a_delta_was_observed() {
    assert_eq!(
        format_model_stage(&stage(Some(Duration::from_millis(250)))),
        "Model stage: turn=1 completed=true elapsed=Some(900ms) first_text=250ms usage=unavailable"
    );
}

#[test]
fn model_stage_line_marks_time_to_first_text_unavailable_when_no_text_streamed() {
    assert_eq!(
        format_model_stage(&stage(None)),
        "Model stage: turn=1 completed=true elapsed=Some(900ms) first_text=unavailable usage=unavailable"
    );
}

#[test]
fn first_audio_line_distinguishes_a_measurement_from_unavailable() {
    assert_eq!(
        format_first_audio(Some(Duration::from_millis(3400))),
        "Speech diagnostics (previous answer): time_to_first_audio=3.4s"
    );
    assert_eq!(
        format_first_audio(None),
        "Speech diagnostics (previous answer): time_to_first_audio=unavailable"
    );
}

#[derive(Debug, thiserror::Error)]
enum Outer {
    #[error("speech synthesis failed")]
    Synthesis(#[source] Inner),
    #[error("speech is unavailable")]
    Plain,
}

#[derive(Debug, thiserror::Error)]
#[error("tts synthesis timed out")]
struct Inner;

#[test]
fn a_warning_names_every_cause_in_the_error_chain() {
    let cases = [
        (
            Outer::Synthesis(Inner),
            "speech synthesis failed: tts synthesis timed out",
        ),
        (Outer::Plain, "speech is unavailable"),
    ];
    for (error, expected) in cases {
        assert_eq!(error_chain(&error), expected);
    }
}
