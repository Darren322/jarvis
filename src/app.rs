use crate::clients::local_llm::LocalLlm;
use crate::config::AppConfig;
use crate::services::assistant::Assistant;
use crate::tools::system_status::collect_system_status;

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
            .await?;

        println!("Response: {}", response);
        let status = collect_system_status();

        println!("Node: {:?}", status.source_node);
        println!("Uptime: {}", status.formatted_uptime());
        println!(
            "RAM: {:.2} GiB / {:.2} GiB",
            status.used_memory_gib(),
            status.total_memory_gib()
        );
        Ok(())
    }
}
