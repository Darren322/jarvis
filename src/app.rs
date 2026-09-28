use crate::clients::local_llm::LocalLlm;
use crate::config::AppConfig;
use crate::services::assistant::Assistant;

pub struct App {
    assistant: Assistant,
}

impl App {
    pub fn new(config: &AppConfig) -> Result<Self, Box<dyn std::error::Error>> {
        let local_llm = LocalLlm::new(config)?;
        let assistant = Assistant::new(local_llm);

        Ok(Self { assistant })
    }
    pub async fn run(&self) -> Result<(), Box<dyn std::error::Error>> {
        self.assistant.health_check().await?;

        let response = self
            .assistant
            .respond("Reply with Exactly: JARVIS ONLINE")
            //.respond("Use the system_status tool to check the system status.")
            .await?;

        println!("Response: {:#?}", response);
        Ok(())
    }
}
