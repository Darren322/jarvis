mod clients;
mod config;

use config::AppConfig;

use crate::clients::local_llm::LocalLlm;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = AppConfig::load()?;

    // temporary while we're building Slice 1
    let local_llm = LocalLlm::new(&config)?;
    local_llm.health_check().await?;

    let response = local_llm
        .complete("Reply with Exactly: JARVIS ONLINE")
        .await?;

    println!("Response: {}", response);
    Ok(())
}

