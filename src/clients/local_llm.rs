use crate::config::AppConfig;
use rig_core::client::CompletionClient;
use rig_core::completion::AssistantContent;
use rig_core::completion::CompletionModel;
use rig_core::providers::openai;

pub struct LocalLlm {
    model: openai::CompletionModel,
    http_client: reqwest::Client,
    health_url: String,
}

async fn within_deadline<F: std::future::Future>(
    duration: std::time::Duration,
    future: F,
) -> Result<F::Output, tokio::time::error::Elapsed> {
    tokio::time::timeout(duration, future).await
}

impl LocalLlm {
    pub fn new(config: &AppConfig) -> Result<Self, Box<dyn std::error::Error>> {
        let client = openai::CompletionsClient::builder()
            .api_key("local")
            .base_url(&config.local_llm_base_url)
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
    pub async fn complete(&self, prompt: &str) -> Result<String, Box<dyn std::error::Error>> {
        let request = self.model.completion_request(prompt).build();
        let timeout_duration = std::time::Duration::from_secs(30);

        let response = within_deadline(timeout_duration, self.model.completion(request))
            .await
            .map_err(|_| "Local LLM completion timed out after 30s")??;
        let mut output = String::new();
        for content in response.choice {
            match content {
                AssistantContent::Text(text) => {
                    output.push_str(&text.text);
                }
                _ => {}
            }
        }
        if output.is_empty() {
            Err("Local LLM returned no text".into())
        } else {
            Ok(output)
        }
    }
}

#[cfg(test)]
#[path = "tests/local_llm_tests.rs"]
mod local_llm_tests;
