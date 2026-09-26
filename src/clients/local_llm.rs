use crate::config::AppConfig;
use rig_core::providers::openai;

pub async fn health_check(config: &AppConfig) {
    let response = reqwest::get(&config.local_llm_base_url).await;

    match response {
        Ok(res) => {
            if res.status().is_success() {
                println!("jarvis-ai is online");
            } else {
                println!("jarvis-ai is online");
                println!("Status: {}", res.status());
            }
        }

        Err(err) => {
            println!("jarvis-ai is offline");
            println!("Error: {}", err);
        }
    }
}

pub fn local_client(config: &AppConfig) -> openai::Client {
    openai::Client::builder()
        .api_key("local")
        .base_url(&config.local_llm_base_url)
        .build()
        .expect("Failed to crete local LLM client")
}
