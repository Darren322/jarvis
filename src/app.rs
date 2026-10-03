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

        let response = self
            .assistant
            //.respond("Reply with Exactly: JARVIS ONLINE")
            .respond("Use the system_status tool to check the system status.")
            .await?;

        println!("Response: {:#?}", response.output);
        Ok(())
    }
}
