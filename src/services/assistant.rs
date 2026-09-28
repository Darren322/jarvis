use rig_core::{completion::CompletionResponse, message::AssistantContent};

use crate::{
    clients::local_llm::LocalLlm,
    tools::{dispatch, system_status},
};

enum ResponseKind {
    FinalText,
    ToolProposal,
    Rejected,
}
fn classify_response(response: &CompletionResponse) -> ResponseKind {
    let tool_count = response
        .choice
        .iter()
        .filter(|content| matches!(content, AssistantContent::ToolCall(_)))
        .count();
    if tool_count == 0 {
        ResponseKind::FinalText
    } else if tool_count == 1 {
        ResponseKind::ToolProposal
    } else {
        ResponseKind::Rejected
    }
}
pub struct Assistant {
    local_llm: LocalLlm,
}
impl Assistant {
    pub fn new(local_llm: LocalLlm) -> Self {
        Self { local_llm }
    }
    pub async fn respond(
        &self,
        prompt: &str,
    ) -> Result<CompletionResponse, Box<dyn std::error::Error>> {
        let tools = vec![system_status::definition()];
        let response = self.local_llm.complete(prompt, tools).await?;
        let kind = classify_response(&response);

        match kind {
            ResponseKind::FinalText => Ok(response),
            ResponseKind::ToolProposal => self.handle_tool_proposal(response).await,
            ResponseKind::Rejected => Err("response rejected: multiple tool calls proposed".into()),
        }
    }
    pub async fn health_check(&self) -> Result<(), Box<dyn std::error::Error>> {
        self.local_llm.health_check().await
    }
    async fn handle_tool_proposal(
        &self,
        response: CompletionResponse,
    ) -> Result<CompletionResponse, Box<dyn std::error::Error>> {
        let tool_call = response
            .choice
            .iter()
            .find(|content| matches!(content, AssistantContent::ToolCall(_)));
        if let Some(AssistantContent::ToolCall(tool_call)) = tool_call {
            dispatch::validate_tool(&tool_call.function.name, &tool_call.function.arguments)
                .map_err(|err| format!("tool proposal rejected: {err:?}"))?;
        }

        Ok(response)
    }
}
