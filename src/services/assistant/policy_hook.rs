use crate::services::assistant::policy::{PolicyTurn, validate_turn};
use rig_agent::agent::{
    CompletionCallAction, InvalidToolCallAction, ModelTurnAction, RequestPatch, RetryRequest,
    ToolCallAction, hook::AgentHook,
};
use rig_core::{
    message::{AssistantContent, ToolChoice},
    serde_json::{self, json},
};

#[derive(Clone, Default)]
struct ToolExecutionState {
    executions: usize,
}

pub struct JarvisPolicyHook;

impl AgentHook for JarvisPolicyHook {
    // BEFORE LLM request:
    // control what this request is allowed to access

    fn on_completion_call(
        &self,
        ctx: &rig_agent::prelude::HookContext,
        _event: rig_agent::agent::CompletionCallEvent<'_>,
    ) -> impl Future<Output = rig_agent::agent::CompletionCallAction>
    + rig_core::wasm_compat::WasmCompatSend {
        let turn = ctx.turn();

        let action = match turn {
            1 => {
                let patch = RequestPatch::new()
                    .active_tools(["system_status"])
                    .tool_choice(ToolChoice::Auto);
                CompletionCallAction::patch(patch)
            }

            2 => {
                let patch = RequestPatch::new()
                    .active_tools(Vec::<String>::new())
                    .tool_choice(ToolChoice::None);
                CompletionCallAction::patch(patch)
            }

            _ => CompletionCallAction::Stop("Phase 4 only supports two model turns".to_string()),
        };

        async move { action }
    }
    fn on_model_turn_finished(
        &self,
        ctx: &rig_agent::prelude::HookContext,
        event: rig_agent::agent::ModelTurnFinished<'_>,
    ) -> impl Future<Output = rig_agent::agent::ModelTurnAction> + rig_core::wasm_compat::WasmCompatSend
    {
        let has_execution = ctx
            .scratchpad()
            .get::<ToolExecutionState>()
            .is_some_and(|state| state.executions > 0);

        let policy_turn = if has_execution {
            PolicyTurn::AfterTool
        } else {
            PolicyTurn::Initial
        };

        let validation = validate_turn(policy_turn, event.content, event.finish_reason);

        let has_tool_call = event
            .content
            .iter()
            .any(|content| matches!(content, AssistantContent::ToolCall(_)));

        let action = match validation {
            Ok(()) => ModelTurnAction::Continue,

            Err(_) if has_tool_call => {
                // Tool-bearing rejection:
                // let the tool hooks handle it
                ModelTurnAction::Continue
            }

            // Tool-free rejection:
            // discard this response and ask the model again.
            Err(_) => ModelTurnAction::Retry(RetryRequest::Repeat),
        };

        async move { action }
    }

    fn on_tool_call(
        &self,
        ctx: &rig_agent::prelude::HookContext,
        event: rig_agent::agent::ToolCall<'_>,
    ) -> impl Future<Output = rig_agent::agent::ToolCallAction> + rig_core::wasm_compat::WasmCompatSend
    {
        let turn = ctx.turn();

        let valid_args = serde_json::from_str::<serde_json::Value>(event.args)
            .is_ok_and(|args| args == json!({}));

        let allowed = turn == 1 && event.tool_name == "system_status" && valid_args;

        let execution_slot_reserved = if allowed {
            ctx.scratchpad().update::<ToolExecutionState, _>(|state| {
                if state.executions == 0 {
                    state.executions = 1;
                    true
                } else {
                    false
                }
            })
        } else {
            false
        };

        let action = if execution_slot_reserved {
            ToolCallAction::Run
        } else {
            ToolCallAction::Stop("Tool exceution rejected by Phase 4 Policy".to_string())
        };

        async move { action }
    }

    fn on_invalid_tool_call(
        &self,
        _ctx: &rig_agent::prelude::HookContext,
        _event: &rig_agent::agent::InvalidToolCallContext,
    ) -> impl Future<Output = Option<rig_agent::agent::InvalidToolCallAction>>
    + rig_core::wasm_compat::WasmCompatSend {
        async {
            Some(InvalidToolCallAction::Stop {
                reason: "Invalid tool call rejected by Phase 4 policy".to_string(),
            })
        }
    }
}

#[cfg(test)]
#[path = "tests/policy_hook_tests.rs"]
mod policy_hook_tests;
