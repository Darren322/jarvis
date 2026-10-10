use crate::assistant::{AssistantError, CallUsage, RunReport};
use crate::storage::MemoryEnqueueState;

pub(super) fn report_turn_warnings(
    context_warning: Option<&str>,
    archive_error: Option<&crate::storage::StorageError>,
    memory_state: Option<MemoryEnqueueState>,
) {
    if let Some(warning) = context_warning {
        eprintln!("Warning: {warning}");
    }
    if let Some(error) = archive_error {
        eprintln!("Warning: this turn could not be saved to the local archive: {error}");
    }
    if matches!(memory_state, Some(MemoryEnqueueState::Pending)) {
        eprintln!(
            "Memory extraction is pending. Check /memory status for backlog or retry details."
        );
    }
}

pub(super) fn safe_error_message(error: &AssistantError) -> &'static str {
    match error {
        #[cfg(test)]
        AssistantError::Prompt(_) => "The model request failed.",
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
