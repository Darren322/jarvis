use crate::config::AppConfig;
use rig_core::client::CompletionClient;
use rig_core::providers::openai;

// LEARNING: Keep the completion model and health-check HTTP client separate:
// they call different endpoints and use different timeout policies.
pub struct LocalLlm {
    model: openai::CompletionModel,
    http_client: reqwest::Client,
    health_url: String,
}

impl LocalLlm {
    // LEARNING: Rig 0.42.0's `CompletionModel` is `Clone`, so this returns an
    // owned clone of the model handle without consuming the stored field.
    pub fn model(&self) -> openai::CompletionModel {
        self.model.clone()
    }
    pub fn new(config: &AppConfig) -> Result<Self, Box<dyn std::error::Error>> {
        // LEARNING: These builders configure the provider client and HTTP
        // client; they do not build a Rig agent or execute a model request.
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

        // LEARNING: Health checks use their own shorter timeout; model HTTP
        // uses 30 seconds with retries disabled. A timeout still cannot prove
        // that a remote server stopped processing the request.
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
