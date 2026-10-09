use super::error::AssistantError;
use rig_agent::agent::PromptResponse;
use rig_core::completion::Usage;
use std::time::Duration;

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RunReport {
    pub(crate) outcome: RunOutcome,
    pub(crate) elapsed: Duration,
    /// Whether observer hooks could read and finalize their shared state.
    /// Poisoned state is discarded and reported as unavailable.
    pub(crate) observations_available: bool,
    pub(crate) model_stages: Vec<ModelStage>,
    pub(crate) tool_stages: Vec<ToolStage>,
}

#[derive(Debug)]
pub(crate) struct AssistantRun {
    pub(crate) response: PromptResponse,
    pub(crate) report: RunReport,
}

#[derive(Debug)]
pub(crate) struct AssistantRunError {
    pub(crate) error: AssistantError,
    pub(crate) report: RunReport,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum CallUsage {
    Unavailable,
    Normalized(Usage),
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ModelStage {
    pub(crate) turn: usize,
    /// True when Rig delivered the matching `ModelTurnFinished` event. This
    /// records a completed model call even if Jarvis policy later rejects it.
    pub(crate) completed: bool,
    /// Host-side elapsed time from this observer's completion-call hook to
    /// `ModelTurnFinished`. This includes Rig request preparation and model
    /// dispatch, so it is not provider-only latency.
    pub(crate) elapsed: Option<Duration>,
    pub(crate) usage: CallUsage,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RunOutcome {
    Success,
    TimedOut,
    InvalidResponse,
    PromptFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToolStageOutcome {
    Incomplete,
    Success,
    Error,
    Refused,
    Skipped,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ToolStage {
    pub(crate) turn: usize,
    pub(crate) internal_call_id: String,
    pub(crate) name: String,
    pub(crate) outcome: ToolStageOutcome,
    /// Host-side elapsed time from this observer's pre-tool hook to Rig's
    /// result hook, including dispatch and tool execution.
    pub(crate) elapsed: Option<Duration>,
}

impl RunReport {
    pub(crate) fn observed(
        outcome: RunOutcome,
        elapsed: Duration,
        model_stages: Vec<ModelStage>,
        tool_stages: Vec<ToolStage>,
    ) -> Self {
        Self {
            outcome,
            elapsed,
            observations_available: true,
            model_stages,
            tool_stages,
        }
    }

    pub(crate) fn unavailable(outcome: RunOutcome, elapsed: Duration) -> Self {
        Self {
            outcome,
            elapsed,
            observations_available: false,
            model_stages: Vec::new(),
            tool_stages: Vec::new(),
        }
    }
}

pub(crate) fn classify_usage(usage: Usage) -> CallUsage {
    if usage.has_values() {
        CallUsage::Normalized(usage)
    } else {
        CallUsage::Unavailable
    }
}

impl std::fmt::Display for AssistantRunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.error)
    }
}

impl std::error::Error for AssistantRunError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}
