use crate::config::AppConfig;
use rig_core::client::CompletionClient;
use rig_core::providers::openai;

pub struct LocalLlm {
    model: openai::CompletionModel,
    http_client: reqwest::Client,
    health_url: String,
}

impl LocalLlm {
    pub fn model(&self) -> openai::CompletionModel {
        self.model.clone()
    }
    pub fn new(config: &AppConfig) -> Result<Self, Box<dyn std::error::Error>> {
        let generation_http_client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .retry(reqwest::retry::never())
            .redirect(reqwest::redirect::Policy::none())
            .build()?;

        let client = openai::CompletionsClient::builder()
            .api_key("local")
            .base_url(&config.local_llm_base_url)
            .http_client(generation_http_client)
            .build()?;

        let model = client.completion_model(&config.local_llm_model);

        let http_client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(3))
            .build()?;

        Ok(Self {
            model,
            http_client,
            health_url: config.local_llm_health_url.clone(),
        })
    }
    pub async fn health_check(&self) -> Result<(), Box<dyn std::error::Error>> {
        // await?  <- ? if Ok helps to unwrap the value and keep going. If Err, immediately return that error from the current function
        let response = self.http_client.get(&self.health_url).send().await?;
        // returns Err if 404/500/503 etc.
        response.error_for_status()?;
        Ok(())
    }
}
