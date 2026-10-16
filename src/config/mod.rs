mod embedding;
mod local_llm;
mod tts;

use std::{
    env, io,
    path::{Path, PathBuf},
};

pub use embedding::EmbeddingConfig;
pub use local_llm::LocalLlmConfig;
pub use tts::{OptionalTtsConfig, TtsConfig};

const DEFAULT_ARCHIVE_PATH: &str = "data/jarvis.sqlite3";

/// Process settings. Every path is absolute: relative values are resolved
/// against the working directory once, at load time.
pub struct AppConfig {
    pub local_llm: LocalLlmConfig,
    pub embedding: EmbeddingConfig,
    pub archive_path: PathBuf,
    pub tts: OptionalTtsConfig,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("required environment variable {name} is missing or not valid Unicode")]
    Var {
        name: &'static str,
        #[source]
        source: env::VarError,
    },
    #[error("could not resolve the current working directory")]
    CurrentDir(#[source] io::Error),
}

impl AppConfig {
    pub fn load() -> Result<Self, ConfigError> {
        dotenvy::dotenv().ok();

        // Required model-server settings fail startup. Optional speech problems are
        // represented in `tts` and leave text chat available.
        let local_llm = LocalLlmConfig::from_env()?;
        let embedding = EmbeddingConfig::from_env()?;
        let current_dir = env::current_dir().map_err(ConfigError::CurrentDir)?;
        let archive_path = resolve_from(
            &current_dir,
            configured_path(env::var_os("JARVIS_ARCHIVE_PATH"), DEFAULT_ARCHIVE_PATH),
        );

        Ok(Self {
            local_llm,
            embedding,
            archive_path,
            tts: OptionalTtsConfig::from_env(&current_dir),
        })
    }
}

fn required_env(name: &'static str) -> Result<String, ConfigError> {
    env::var(name).map_err(|source| ConfigError::Var { name, source })
}

fn configured_path(value: Option<std::ffi::OsString>, default: &str) -> PathBuf {
    value
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(default))
}

fn optional_env(name: &str) -> Result<Option<String>, env::VarError> {
    match env::var(name) {
        Ok(value) => Ok(Some(value)),
        Err(env::VarError::NotPresent) => Ok(None),
        Err(error) => Err(error),
    }
}

fn resolve_from(current_dir: &Path, path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        path
    } else {
        current_dir.join(path)
    }
}

#[cfg(test)]
#[path = "../../tests/unit/config/config_tests.rs"]
mod tests;
