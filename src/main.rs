mod app;
mod clients;
mod config;
mod services;
mod storage;
mod tools;

use std::process::ExitCode;

use crate::app::App;
use config::AppConfig;

// LEARNING: `#[tokio::main]` starts an async runtime around this entry point.
// An `async fn` creates a future; `.await` can suspend this task for the
// runtime, though an already-ready future may finish immediately. This differs
// from blocking Java `Future.get()`.
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

// LEARNING: `Result<T, E>` stores `Ok(T)` or `Err(E)`; `?` returns an error
// early, similar to exception propagation, but it remains a value to handle.
// Here `()` means no meaningful success value (like Java `void`).
// `Box<dyn Error>` owns a concrete error behind a trait-object interface,
// allowing different error types in this return position.
async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let config = AppConfig::load()?;
    let app = App::new(&config)?;
    app.run().await?;
    // LEARNING: Without a semicolon, this final `Ok(())` is returned by `run`.
    Ok(())
}
