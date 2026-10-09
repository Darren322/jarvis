use crate::services::assistant::run_report::{
    CallUsage, ModelStage, RunOutcome, RunReport, ToolStage, ToolStageOutcome, classify_usage,
};

use rig_agent::agent::{
    ToolResultEvent,
    hook::{
        AgentHook, CompletionCall, CompletionCallAction, HookContext, ModelTurnAction,
        ModelTurnFinished, ToolCall, ToolCallAction, ToolResultAction,
    },
};
use std::{
    collections::HashMap,
    mem,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Clone, Default)]
pub(crate) struct RunObserver {
    observed: Arc<Mutex<ObservedRun>>,
}

#[derive(Default)]
struct ObservedRun {
    model_starts: HashMap<usize, Instant>,
    tool_starts: HashMap<String, ToolStart>,
    model_stages: Vec<ModelStage>,
    tool_stages: Vec<ToolStage>,
}

struct ToolStart {
    turn: usize,
    name: String,
    started_at: Instant,
}

impl RunObserver {
    /// Finalize the observations after Rig has dropped its hook clone.
    ///
    /// A poisoned mutex makes every value inside it unreliable, so this path
    /// reports observations as unavailable without recovering or exposing the
    /// poisoned measurements.
    pub(crate) fn finish(self, outcome: RunOutcome, elapsed: Duration) -> RunReport {
        let observed = match self.observed.lock() {
            Ok(mut observed) => mem::take(&mut *observed),
            Err(_) => return RunReport::unavailable(outcome, elapsed),
        };

        observed.finish(outcome, elapsed)
    }
}

impl ObservedRun {
    fn start_tool(
        &mut self,
        turn: usize,
        internal_call_id: String,
        name: String,
        started_at: Instant,
    ) {
        self.tool_starts.insert(
            internal_call_id,
            ToolStart {
                turn,
                name,
                started_at,
            },
        );
    }

    fn finish_tool(
        &mut self,
        turn: usize,
        internal_call_id: String,
        name: String,
        outcome: ToolStageOutcome,
        finished_at: Instant,
    ) {
        let start = self.tool_starts.remove(&internal_call_id);
        let (turn, name, elapsed) = match start {
            Some(start) => (
                start.turn,
                start.name,
                Some(finished_at.saturating_duration_since(start.started_at)),
            ),
            None => (turn, name, None),
        };

        self.tool_stages.push(ToolStage {
            turn,
            internal_call_id,
            name,
            outcome,
            elapsed,
        });
    }

    fn finish(mut self, outcome: RunOutcome, elapsed: Duration) -> RunReport {
        for (turn, _) in self.model_starts.drain() {
            self.model_stages.push(ModelStage {
                turn,
                completed: false,
                elapsed: None,
                usage: CallUsage::Unavailable,
            });
        }

        for (internal_call_id, start) in self.tool_starts.drain() {
            self.tool_stages.push(ToolStage {
                turn: start.turn,
                internal_call_id,
                name: start.name,
                outcome: ToolStageOutcome::Incomplete,
                elapsed: None,
            });
        }

        self.model_stages.sort_by_key(|stage| stage.turn);
        self.tool_stages.sort_by(|left, right| {
            left.turn
                .cmp(&right.turn)
                .then_with(|| left.internal_call_id.cmp(&right.internal_call_id))
                .then_with(|| left.name.cmp(&right.name))
        });

        RunReport::observed(outcome, elapsed, self.model_stages, self.tool_stages)
    }
}

impl AgentHook for RunObserver {
    async fn on_completion_call(
        &self,
        ctx: &HookContext,
        _event: CompletionCall<'_>,
    ) -> CompletionCallAction {
        if let Ok(mut observed) = self.observed.lock() {
            observed.model_starts.insert(ctx.turn(), Instant::now());
        }

        CompletionCallAction::Continue
    }

    async fn on_model_turn_finished(
        &self,
        _ctx: &HookContext,
        event: ModelTurnFinished<'_>,
    ) -> ModelTurnAction {
        let usage = classify_usage(event.usage);

        if let Ok(mut observed) = self.observed.lock() {
            let elapsed = observed
                .model_starts
                .remove(&event.turn)
                .map(|started_at| started_at.elapsed());
            observed.model_stages.push(ModelStage {
                turn: event.turn,
                completed: true,
                elapsed,
                usage,
            });
        }

        // LEARNING: Rig raises this common lifecycle event after it has
        // assembled a model turn on both its blocking and streaming paths.
        // Returning `Continue` keeps this observer passive so later policy
        // hooks can still accept or reject the turn.
        ModelTurnAction::Continue
    }

    async fn on_tool_call(&self, ctx: &HookContext, event: ToolCall<'_>) -> ToolCallAction {
        if let Ok(mut observed) = self.observed.lock() {
            observed.start_tool(
                ctx.turn(),
                event.internal_call_id.to_string(),
                event.tool_name.to_string(),
                Instant::now(),
            );
        }

        ToolCallAction::Run
    }

    async fn on_tool_result(
        &self,
        ctx: &HookContext,
        event: ToolResultEvent<'_>,
    ) -> ToolResultAction {
        if let Ok(mut observed) = self.observed.lock() {
            observed.finish_tool(
                ctx.turn(),
                event.internal_call_id.to_string(),
                event.tool_name.to_string(),
                classify_tool_result(event.raw_result),
                Instant::now(),
            );
        }

        ToolResultAction::Keep
    }
}

fn classify_tool_result(result: &rig_core::tool::ToolResult) -> ToolStageOutcome {
    // Rig 0.42 keeps its disposition enum private but exposes these four
    // mutually exclusive predicates. The final branch is the only remaining
    // disposition in this pinned version: Skipped.
    if result.is_success() {
        ToolStageOutcome::Success
    } else if result.is_error() {
        ToolStageOutcome::Error
    } else if result.is_refused() {
        ToolStageOutcome::Refused
    } else {
        ToolStageOutcome::Skipped
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/services/assistant/run_observer_tests.rs"]
mod tests;
