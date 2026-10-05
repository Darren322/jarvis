use std::{env, path::PathBuf};

const DEFAULT_ARCHIVE_PATH: &str = "data/jarvis.sqlite3";

pub struct AppConfig {
    pub local_llm_base_url: String,
    pub local_llm_health_url: String,
    pub local_llm_model: String,
    pub archive_path: String,
    pub tts: OptionalTtsConfig,
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
    pub fn load() -> Result<Self, env::VarError> {
        dotenvy::dotenv().ok();

        // Keep required LLM settings first so their existing VarError behavior
        // remains unchanged. Optional speech problems are represented in `tts`
        // and leave text chat available.
        let local_llm_base_url = env::var("LOCAL_LLM_BASE_URL")?;
        let local_llm_health_url = env::var("LOCAL_LLM_HEALTH_URL")?;
        let local_llm_model = env::var("LOCAL_LLM_MODEL")?;
        let archive_path =
            env::var("JARVIS_ARCHIVE_PATH").unwrap_or_else(|_| DEFAULT_ARCHIVE_PATH.to_string());

        Ok(Self {
            local_llm_base_url,
            local_llm_health_url,
            local_llm_model,
            archive_path,
            tts: load_tts_config(),
        })
    }
}

fn load_tts_config() -> OptionalTtsConfig {
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

    let current_dir = match env::current_dir() {
        Ok(path) => path,
        Err(_) => {
            return OptionalTtsConfig::Invalid(
                "speech paths could not be resolved from the current directory.",
            );
        }
    };

    let worker_script = match optional_env("JARVIS_TTS_WORKER") {
        Ok(None) => current_dir.join("scripts/tts/supertonic_worker.py"),
        Ok(Some(value)) if !value.is_empty() => resolve_from(&current_dir, PathBuf::from(value)),
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
        model_dir: resolve_from(&current_dir, model_dir),
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

fn resolve_from(current_dir: &std::path::Path, path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        path
    } else {
        current_dir.join(path)
    }
}
