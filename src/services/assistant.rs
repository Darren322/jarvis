use crate::clients::local_llm::LocalLlm;

pub struct Assistant {
    local_llm: LocalLlm,
}
impl Assistant {
    pub fn new(local_llm: LocalLlm) -> Self {
        Self { local_llm }
    }
    pub async fn respond(&self, prompt: &str) -> Result<String, Box<dyn std::error::Error>> {
        self.local_llm.complete(prompt).await
    }
    pub async fn health_check(&self) -> Result<(), Box<dyn std::error::Error>> {
        self.local_llm.health_check().await
    }
}
