use crate::config::AppConfig;
use rig_core::client::CompletionClient;
use rig_core::completion::AssistantContent;
use rig_core::completion::CompletionModel;
use rig_core::providers::openai;

pub struct LocalLlm {
    model: openai::CompletionModel,
    health_url: String,
}

impl LocalLlm {
    pub fn new(config: &AppConfig) -> Self {
        let client = openai::CompletionsClient::builder()
            .api_key("local")
            .base_url(&config.local_llm_base_url)
            .build()
            .expect("Failed to crete local LLM client");

        let model = client.completion_model(&config.local_llm_model);

        Self {
            model,
            health_url: config.local_llm_health_url.clone(),
        }
    }
    pub async fn health_check(&self) {
        let response = reqwest::get(&self.health_url).await;

        match response {
            Ok(res) => {
                if res.status().is_success() {
                    println!("jarvis-ai is online");
                } else {
                    println!("jarvis-ai health check failed");
                    println!("Status: {}", res.status());
                }
            }

            Err(err) => {
                println!("jarvis-ai is offline");
                println!("Error: {}", err);
            }
        }
    }
    pub async fn complete(&self, prompt: &str) -> Result<String, Box<dyn std::error::Error>> {
        let request = self.model.completion_request(prompt).build();
        let response = self.model.completion(request).await?;
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
