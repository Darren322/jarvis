use crate::clients::local_llm::LocalLlm;
use crate::config::AppConfig;

pub struct App {
    local_llm: LocalLlm,
}

impl App {
    pub fn new(config: &AppConfig) -> Result<Self, Box<dyn std::error::Error>> {
        let local_llm = LocalLlm::new(config)?;

        Ok(Self { local_llm })
    }
    pub async fn run(&self) -> Result<(), Box<dyn std::error::Error>> {
        self.local_llm.health_check().await?;

        let response = self
            .local_llm
            .complete("Reply with Exactly: JARVIS ONLINE")
            .await?;

        println!("Response: {}", response);
        Ok(())
    }
}
