use std::{
    env, io,
    path::{Path, PathBuf},
};

const DEFAULT_ARCHIVE_PATH: &str = "data/jarvis.sqlite3";
const DEFAULT_EMBEDDING_MODEL_DIR: &str = "data/embeddings";

/// Process settings. Every path is absolute: relative values are resolved
/// against the working directory once, at load time.
pub struct AppConfig {
    pub local_llm: LocalLlmConfig,
    pub archive_path: PathBuf,
    pub embedding_model_dir: PathBuf,
    pub tts: OptionalTtsConfig,
}

pub struct LocalLlmConfig {
    pub base_url: String,
    pub health_url: String,
    pub model: String,
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

#[derive(Clone, Debug)]
pub enum OptionalTtsConfig {
    Disabled,
    Invalid(&'static str),
    Configured(TtsConfig),
}

#[derive(Clone, Debug)]
pub struct TtsConfig {
    pub python: PathBuf,
    pub worker_script: PathBuf,
    pub model_dir: PathBuf,
    pub threads: u32,
    pub audio_device: Option<String>,
}

impl AppConfig {
    pub fn load() -> Result<Self, ConfigError> {
        dotenvy::dotenv().ok();

        // Required LLM settings fail startup. Optional speech problems are
        // represented in `tts` and leave text chat available.
        let local_llm = LocalLlmConfig {
            base_url: required_env("LOCAL_LLM_BASE_URL")?,
            health_url: required_env("LOCAL_LLM_HEALTH_URL")?,
            model: required_env("LOCAL_LLM_MODEL")?,
        };
        let current_dir = env::current_dir().map_err(ConfigError::CurrentDir)?;
        let path =
            |name, default| resolve_from(&current_dir, configured_path(env::var_os(name), default));

        Ok(Self {
            local_llm,
            archive_path: path("JARVIS_ARCHIVE_PATH", DEFAULT_ARCHIVE_PATH),
            embedding_model_dir: path("JARVIS_EMBEDDING_MODEL_DIR", DEFAULT_EMBEDDING_MODEL_DIR),
            tts: load_tts_config(&current_dir),
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

fn load_tts_config(current_dir: &Path) -> OptionalTtsConfig {
    let model_dir = match optional_env("JARVIS_TTS_MODEL_DIR") {
        Ok(None) => return OptionalTtsConfig::Disabled,
        Ok(Some(value)) if !value.is_empty() => PathBuf::from(value),
        Ok(Some(_)) => {
            return OptionalTtsConfig::Invalid("JARVIS_TTS_MODEL_DIR cannot be empty.");
        }
        Err(_) => {
            return OptionalTtsConfig::Invalid("JARVIS_TTS_MODEL_DIR is not valid Unicode.");
        }
    };

    let python = match env::var("JARVIS_TTS_PYTHON") {
        Ok(value) if !value.is_empty() => PathBuf::from(value),
        Ok(_) => {
            return OptionalTtsConfig::Invalid("JARVIS_TTS_PYTHON cannot be empty.");
        }
        Err(env::VarError::NotPresent) => {
            return OptionalTtsConfig::Invalid(
                "JARVIS_TTS_PYTHON is required when speech is enabled.",
            );
        }
        Err(env::VarError::NotUnicode(_)) => {
            return OptionalTtsConfig::Invalid("JARVIS_TTS_PYTHON is not valid Unicode.");
        }
    };
    if !python.is_absolute() {
        return OptionalTtsConfig::Invalid("JARVIS_TTS_PYTHON must be an absolute path.");
    }

    let worker_script = match optional_env("JARVIS_TTS_WORKER") {
        Ok(None) => current_dir.join("scripts/tts/supertonic_worker.py"),
        Ok(Some(value)) if !value.is_empty() => resolve_from(current_dir, PathBuf::from(value)),
        Ok(Some(_)) => {
            return OptionalTtsConfig::Invalid("JARVIS_TTS_WORKER cannot be empty.");
        }
        Err(_) => {
            return OptionalTtsConfig::Invalid("JARVIS_TTS_WORKER is not valid Unicode.");
        }
    };

    let threads = match env::var("JARVIS_TTS_THREADS") {
        Ok(value) => match value.parse::<u32>() {
            Ok(threads @ 1..=4) => threads,
            _ => {
                return OptionalTtsConfig::Invalid(
                    "JARVIS_TTS_THREADS must be an integer from 1 through 4.",
                );
            }
        },
        Err(env::VarError::NotPresent) => 2,
        Err(env::VarError::NotUnicode(_)) => {
            return OptionalTtsConfig::Invalid("JARVIS_TTS_THREADS is not valid Unicode.");
        }
    };

    let audio_device = match optional_env("JARVIS_AUDIO_DEVICE") {
        Ok(None) => None,
        Ok(Some(value)) if !value.is_empty() => Some(value),
        Ok(Some(_)) => {
            return OptionalTtsConfig::Invalid("JARVIS_AUDIO_DEVICE cannot be empty.");
        }
        Err(_) => {
            return OptionalTtsConfig::Invalid("JARVIS_AUDIO_DEVICE is not valid Unicode.");
        }
    };

    OptionalTtsConfig::Configured(TtsConfig {
        python,
        worker_script,
        model_dir: resolve_from(current_dir, model_dir),
        threads,
        audio_device,
    })
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
