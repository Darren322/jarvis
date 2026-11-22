use crate::features::assistant::ToolUse;
use crate::features::assistant::policy::{PolicyTurn, validate_turn};
use crate::tools::ALLOWED_TOOLS;
use rig_agent::agent::{
    CompletionCallAction, InvalidToolCallAction, ModelTurnAction, RequestPatch, hook::AgentHook,
};
use rig_core::message::ToolChoice;

pub(super) struct JarvisPolicyHook {
    pub(super) tools: ToolUse,
}

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
            // A run that may call no tool keeps the list too, for the same cache
            // reason as the second request below (timers D20).
            1 => {
                let choice = match self.tools {
                    ToolUse::Offered => ToolChoice::Auto,
                    ToolUse::Forbidden => ToolChoice::None,
                };
                let patch = RequestPatch::new()
                    .active_tools(ALLOWED_TOOLS.iter().map(|rule| rule.name))
                    .tool_choice(choice);
                CompletionCallAction::patch(patch)
            }

            // The second request still lists the tools, so its prompt starts
            // exactly like the first one and the server reuses its cache
            // (fast-memory 10: without the list, every tool run re-read the whole
            // conversation, about 10 s). `ToolChoice::None` is what forbids a call.
            2 => {
                let patch = RequestPatch::new()
                    .active_tools(ALLOWED_TOOLS.iter().map(|rule| rule.name))
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
        let policy_turn = match (event.turn, self.tools) {
            (1, ToolUse::Offered) => PolicyTurn::Initial,
            (1, ToolUse::Forbidden) => PolicyTurn::ToolsForbidden,
            (2, _) => PolicyTurn::AfterTool,
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
