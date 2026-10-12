use crate::features::assistant::{AssistantError, CallUsage, RunReport};
use crate::storage::MemorySaveState;

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
        eprintln!(
            "Model stage: turn={} completed={} elapsed={:?} usage={usage}",
            stage.turn, stage.completed, stage.elapsed
        );
    }
    for stage in &report.tool_stages {
        eprintln!(
            "Tool stage: turn={} internal_call_id={:?} name={:?} outcome={:?} elapsed={:?}",
            stage.turn, stage.internal_call_id, stage.name, stage.outcome, stage.elapsed
        );
    }
}
