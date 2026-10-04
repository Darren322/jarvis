use super::error::AssistantError;
use rig_agent::agent::PromptResponse;
use rig_core::completion::Usage;
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

#[derive(Debug)]
pub(crate) struct RunReport {
    pub(crate) outcome: RunOutcome,
    pub(crate) elapsed: Duration,
    pub(crate) model_stages: Vec<ModelStage>,
    pub(crate) tool_stages: Vec<ToolStage>,
}

#[derive(Debug)]
pub(crate) struct ObservedRun {
    pub(crate) model_starts: HashMap<usize, Instant>,
    pub(crate) tool_starts: HashMap<String, ToolStart>,
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
#[derive(Debug)]
pub(crate) enum CallUsage {
    Unavailable,
    Normalized(Usage),
}

#[derive(Debug)]
pub(crate) struct ModelStage {
    pub(crate) turn: usize,
    pub(crate) completed: bool,
    pub(crate) elapsed: Option<Duration>,
    pub(crate) usage: CallUsage,
}

#[derive(Debug)]
pub(crate) enum RunOutcome {
    Success,
    TimedOut,
    InvalidResponse,
    PromptFailed,
}

#[derive(Debug)]
pub(crate) enum ToolStageOutcome {
    Incomplete,
    Success,
    Error,
    Refused,
    Skipped,
}

#[derive(Debug)]
pub(crate) struct ToolStart {
    pub(crate) name: String,
    pub(crate) started_at: Instant,
}

#[derive(Debug)]
pub(crate) struct ToolStage {
    pub(crate) name: String,
    pub(crate) outcome: ToolStageOutcome,
    pub(crate) elapsed: Option<Duration>,
}

pub(crate) fn classify_usage(usage: Usage) -> CallUsage {
    if usage.has_values() {
        CallUsage::Normalized(usage)
    } else {
        CallUsage::Unavailable
    }
}

impl ObservedRun {
    pub(crate) fn new() -> Self {
        Self {
            model_starts: HashMap::new(),
            tool_starts: HashMap::new(),
            model_stages: Vec::new(),
            tool_stages: Vec::new(),
        }
    }
    pub(crate) fn finalize_incomplete_stages(&mut self) {
        for (turn, _) in self.model_starts.drain() {
            self.model_stages.push(ModelStage {
                turn,
                completed: false,
                elapsed: None,
                usage: CallUsage::Unavailable,
            })
        }

        for (_, start) in self.tool_starts.drain() {
            self.tool_stages.push(ToolStage {
                name: start.name,
                outcome: ToolStageOutcome::Incomplete,
                elapsed: None,
            })
        }
    }

    pub(crate) fn finish(mut self, outcome: RunOutcome, elapsed: Duration) -> RunReport {
        self.finalize_incomplete_stages();

        RunReport {
            outcome,
            elapsed,
            model_stages: self.model_stages,
            tool_stages: self.tool_stages,
        }
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
