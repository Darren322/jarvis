use crate::features::assistant::policy::{PolicyTurn, validate_turn};
use crate::tools::ALLOWED_TOOLS;
use rig_agent::agent::{
    CompletionCallAction, InvalidToolCallAction, ModelTurnAction, RequestPatch, hook::AgentHook,
};
use rig_core::message::ToolChoice;

pub(super) struct JarvisPolicyHook;

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
                    .active_tools(ALLOWED_TOOLS.iter().map(|rule| rule.name))
                    .tool_choice(ToolChoice::Auto);
                CompletionCallAction::patch(patch)
            }

            2 => {
                let patch = RequestPatch::new()
                    .active_tools(Vec::<String>::new())
                    .tool_choice(ToolChoice::None);
                CompletionCallAction::patch(patch)
            }

            _ => CompletionCallAction::Stop(
                "Jarvis policy only supports two model turns".to_string(),
            ),
        };

        async move { action }
    }
    async fn on_model_turn_finished(
        &self,
        _ctx: &rig_agent::prelude::HookContext,
        event: rig_agent::agent::ModelTurnFinished<'_>,
    ) -> rig_agent::agent::ModelTurnAction {
        let policy_turn = match event.turn {
            1 => PolicyTurn::Initial,
            2 => PolicyTurn::AfterTool,
            _ => return ModelTurnAction::stop("Unexpected model turn"),
        };

        match validate_turn(policy_turn, event.content, event.finish_reason) {
            Ok(()) => ModelTurnAction::Continue,
            Err(reason) => ModelTurnAction::stop(format!("Jarvis policy: {reason:?}")),
        }
    }

    async fn on_invalid_tool_call(
        &self,
        _ctx: &rig_agent::prelude::HookContext,
        _event: &rig_agent::agent::InvalidToolCallContext,
    ) -> Option<rig_agent::agent::InvalidToolCallAction> {
        Some(InvalidToolCallAction::Stop {
            reason: "Invalid tool call rejected by Jarvis policy".to_string(),
        })
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/features/assistant/policy_hook_tests.rs"]
mod tests;
