use rig_core::completion::CompletionResponse;

use crate::{clients::local_llm::LocalLlm, tools::system_status};

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
        self.local_llm.complete(prompt, tools).await
    }
    pub async fn health_check(&self) -> Result<(), Box<dyn std::error::Error>> {
        self.local_llm.health_check().await
    }
}
