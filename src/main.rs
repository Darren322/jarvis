mod clients;
mod config;

use config::AppConfig;

use crate::clients::local_llm;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = AppConfig::load()?;

    // temporary while we're building Slice 1
    local_llm::health_check(&config).await;

    Ok(())
}
