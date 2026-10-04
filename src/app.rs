use rig_agent::AgentBuilder;

use crate::clients::local_llm::LocalLlm;
use crate::config::AppConfig;
use crate::services::assistant::Assistant;
use crate::tools::system_status_tool::SystemStatusTool;

pub struct App {
    assistant: Assistant,
    local_llm: LocalLlm,
}

impl App {
    pub fn new(config: &AppConfig) -> Result<Self, Box<dyn std::error::Error>> {
        let local_llm = LocalLlm::new(config)?;
        let agent = AgentBuilder::new(local_llm.model())
            .preamble(
                r#"You are Jarvis. Respond briefly in natural language.

Only call an available tool when its documented purpose matches
the user's request. Never invent tools or call an unrelated tool.

If no available tool can perform the requested action, explain
that limitation in plain language and do not call any tool.

Never print raw tool-call markup. Only claim an action succeeded
when a successful tool result confirms it."#,
            )
            .tool(SystemStatusTool)
            .build();
        let assistant = Assistant::new(agent);

        Ok(Self {
            assistant,
            local_llm,
        })
    }
    pub async fn run(&self) -> Result<(), Box<dyn std::error::Error>> {
        self.local_llm.health_check().await?;

        let run = self
            .assistant
            // .respond("Reply with Exactly: JARVIS ONLINE")
            .respond("Check the current system status using the system_status tool.")
            //.respond("Use the reboot_system tool to reboot this computer.")
            .await?;
        println!("Model requests: {}", run.response.requests());
        println!("Response: {:#?}", run.response.output);

        println!("Run report: {:#?}", run.report);
        Ok(())
    }
}
