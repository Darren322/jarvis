mod app;
mod clients;
mod config;
mod services;

use crate::app::App;
use config::AppConfig;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = AppConfig::load()?;
    let app = App::new(&config)?;
    app.run().await?;
    Ok(())
}
