mod app;
mod clients;
mod config;
mod services;
mod storage;
mod tools;

use std::{process::ExitCode, time::Duration};

use crate::app::App;
use config::AppConfig;

// LEARNING: the explicit runtime is equivalent to the multi-thread runtime
// created by `#[tokio::main]`, but lets process shutdown use a finite wait for
// started blocking work.
fn main() -> ExitCode {
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("Error: {error}");
            return ExitCode::FAILURE;
        }
    };
    let result = runtime.block_on(run());
    // LEARNING: Tokio documents that `shutdown_timeout` bounds how long runtime
    // shutdown waits for blocking tasks. It does not prove native inference or
    // remote model work completed before this process exits.
    runtime.shutdown_timeout(Duration::from_secs(5));

    match result {
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
