use rig_agent::{Agent, agent::PromptResponse};

mod error;
mod policy;
pub use error::AssistantError;

pub struct Assistant {
    agent: Agent,
}
impl Assistant {
    pub fn new(agent: Agent) -> Self {
        Self { agent }
    }
    pub async fn respond(&self, prompt: &str) -> Result<PromptResponse, AssistantError> {
        let run = self
            .agent
            .runner(prompt)
            .max_turns(1)
            .max_invalid_tool_call_retries(0)
            .without_memory()
            .run();

        let response = within_deadline(std::time::Duration::from_secs(30), run)
            .await
            .map_err(|_| AssistantError::RunTimeout)?
            .map_err(AssistantError::Prompt)?;

        if response.output.trim().is_empty() {
            return Err(AssistantError::InvalidResponse);
        }

        Ok(response)
    }
}
async fn within_deadline<F: std::future::Future>(
    duration: std::time::Duration,
    future: F,
) -> Result<F::Output, tokio::time::error::Elapsed> {
    tokio::time::timeout(duration, future).await
}

#[cfg(test)]
#[path = "tests/assistant_tests.rs"]
mod assistant_tests;
