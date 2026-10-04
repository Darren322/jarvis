use crate::services::assistant::run_report::{
    ModelStage, ToolStage, ToolStageOutcome, ToolStart, classify_usage,
};

use super::run_report::ObservedRun;
use rig_agent::agent::{
    ToolResultEvent,
    hook::{
        AgentHook, CompletionCall, CompletionCallAction, HookContext, ObservationAction, ToolCall,
        ToolCallAction, ToolResultAction,
    },
};
use std::{
    sync::{Arc, Mutex},
    time::Instant,
};

pub(crate) struct RunObserver {
    observed: Arc<Mutex<ObservedRun>>,
}

impl RunObserver {
    pub(crate) fn new(observed: Arc<Mutex<ObservedRun>>) -> Self {
        Self { observed }
    }
}

impl AgentHook for RunObserver {
    fn on_completion_call(
        &self,
        ctx: &HookContext,
        _event: CompletionCall<'_>,
    ) -> impl Future<Output = CompletionCallAction> + rig_core::wasm_compat::WasmCompatSend {
        async move {
            {
                // lock
                let mut observed = self.observed.lock().unwrap();

                observed.model_starts.insert(ctx.turn(), Instant::now());
            } // unlock

            CompletionCallAction::Continue
        }
    }

    fn on_completion_response(
        &self,
        ctx: &HookContext,
        event: rig_agent::agent::CompletionResponseEvent<'_>,
    ) -> impl Future<Output = rig_agent::agent::ObservationAction> + rig_core::wasm_compat::WasmCompatSend
    {
        async move {
            {
                let mut observed = self.observed.lock().unwrap();

                if let Some(start) = observed.model_starts.remove(&ctx.turn()) {
                    observed.model_stages.push(ModelStage {
                        turn: ctx.turn(),
                        completed: true,
                        elapsed: Some(start.elapsed()),
                        usage: classify_usage(event.usage.clone()),
                    });
                }
            }
            ObservationAction::Continue
        }
    }
    fn on_tool_call(
        &self,
        _ctx: &HookContext,
        event: ToolCall<'_>,
    ) -> impl Future<Output = ToolCallAction> + rig_core::wasm_compat::WasmCompatSend {
        async move {
            {
                let mut observed = self.observed.lock().unwrap();

                observed.tool_starts.insert(
                    event.internal_call_id.to_string(),
                    ToolStart {
                        name: event.tool_name.to_string(),
                        started_at: Instant::now(),
                    },
                );
            }

            ToolCallAction::Run
        }
    }

    fn on_tool_result(
        &self,
        _ctx: &HookContext,
        event: ToolResultEvent<'_>,
    ) -> impl Future<Output = ToolResultAction> + rig_core::wasm_compat::WasmCompatSend {
        async move {
            {
                let mut observed = self.observed.lock().unwrap();

                if let Some(start) = observed
                    .tool_starts
                    .remove(&event.internal_call_id.to_string())
                {
                    let outcome = if event.raw_result.is_success() {
                        ToolStageOutcome::Success
                    } else if event.raw_result.is_error() {
                        ToolStageOutcome::Error
                    } else if event.raw_result.is_refused() {
                        ToolStageOutcome::Refused
                    } else {
                        ToolStageOutcome::Skipped
                    };

                    observed.tool_stages.push(ToolStage {
                        name: start.name,
                        outcome,
                        elapsed: Some(start.started_at.elapsed()),
                    });
                }
            }
            ToolResultAction::Keep
        }
    }
}
