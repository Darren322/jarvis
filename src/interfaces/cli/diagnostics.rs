use std::{error::Error, time::Duration};

use crate::features::assistant::{AssistantError, CallUsage, ModelStage, RunReport};
use crate::features::control::Notice;
use crate::storage::MemorySaveState;

/// Joins an error and every `source()` beneath it with `: `, so a warning
/// names the underlying cause (e.g. `speech synthesis failed: tts synthesis
/// timed out`) rather than only the outermost message.
pub(super) fn error_chain(error: &(dyn Error + 'static)) -> String {
    std::iter::successors(Some(error), |&error| error.source())
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(": ")
}

pub(super) fn print_notices(notices: &[Notice]) {
    for notice in notices {
        match notice {
            Notice::Info(text) => println!("{text}"),
            Notice::Warning(text) => eprintln!("Warning: {text}"),
        }
    }
}

pub(super) fn report_turn_warnings(
    context_warning: Option<&str>,
    archive_error: Option<&crate::storage::StorageError>,
    memory_state: Option<MemorySaveState>,
) {
    if let Some(warning) = context_warning {
        eprintln!("Warning: {warning}");
    }
    if let Some(error) = archive_error {
        eprintln!("Warning: this turn could not be saved to the local archive: {error}");
    }
    if matches!(memory_state, Some(MemorySaveState::Suppressed)) {
        eprintln!(
            "This answer used a memory that was forgotten while it ran, so it was not remembered."
        );
    }
}

pub(super) fn safe_error_message(error: &AssistantError) -> &'static str {
    match error {
        AssistantError::Stream(_)
        | AssistantError::MissingFinalResponse
        | AssistantError::DeltaReceiverClosed => "The streamed model response failed.",
        AssistantError::RunTimeout => "The request timed out.",
        AssistantError::InvalidResponse => "The assistant returned an invalid response.",
    }
}

pub(super) fn print_run_diagnostics(report: &RunReport) {
    eprintln!(
        "Run diagnostics: outcome={:?} assistant_elapsed={:?} observations_available={}",
        report.outcome, report.elapsed, report.observations_available
    );
    for stage in &report.model_stages {
        eprintln!("{}", format_model_stage(stage));
    }
    for stage in &report.tool_stages {
        eprintln!(
            "Tool stage: turn={} internal_call_id={:?} name={:?} outcome={:?} elapsed={:?}",
            stage.turn, stage.internal_call_id, stage.name, stage.outcome, stage.elapsed
        );
    }
}

fn format_model_stage(stage: &ModelStage) -> String {
    let usage = match &stage.usage {
        CallUsage::Unavailable => "unavailable".to_string(),
        CallUsage::Normalized(usage) => format!(
            "rig-normalized input={} output={} total={} cached_input={} cache_creation_input={} tool_use_prompt={} reasoning={}",
            usage.input_tokens,
            usage.output_tokens,
            usage.total_tokens,
            usage.cached_input_tokens,
            usage.cache_creation_input_tokens,
            usage.tool_use_prompt_tokens,
            usage.reasoning_tokens,
        ),
    };
    let first_text = match stage.first_text {
        Some(first_text) => format!("{first_text:?}"),
        None => "unavailable".to_string(),
    };
    format!(
        "Model stage: turn={} completed={} elapsed={:?} first_text={first_text} usage={usage}",
        stage.turn, stage.completed, stage.elapsed
    )
}

/// `first_audio` is the time from the prompt being accepted to the speech
/// backend's playback-started signal; `None` when playback never started.
/// The line is printed after the answer's speech ends, often once the next
/// prompt is showing, so it names the answer it belongs to.
pub(super) fn format_first_audio(first_audio: Option<Duration>) -> String {
    match first_audio {
        Some(first_audio) => {
            format!("Speech diagnostics (previous answer): time_to_first_audio={first_audio:?}")
        }
        None => "Speech diagnostics (previous answer): time_to_first_audio=unavailable".to_string(),
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/interfaces/cli/diagnostics_tests.rs"]
mod tests;
