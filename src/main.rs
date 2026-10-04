mod app;
mod clients;
mod config;
mod services;
mod tools;

use std::process::ExitCode;

use crate::app::App;
use config::AppConfig;

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Error: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let config = AppConfig::load()?;
    let app = App::new(&config)?;
    app.run().await?;
    Ok(())
}
