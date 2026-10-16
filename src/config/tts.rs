use std::{
    env,
    path::{Path, PathBuf},
};

use super::{optional_env, resolve_from};

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

impl OptionalTtsConfig {
    pub(super) fn from_env(current_dir: &Path) -> Self {
        let model_dir = match optional_env("JARVIS_TTS_MODEL_DIR") {
            Ok(None) => return Self::Disabled,
            Ok(Some(value)) if !value.is_empty() => PathBuf::from(value),
            Ok(Some(_)) => {
                return Self::Invalid("JARVIS_TTS_MODEL_DIR cannot be empty.");
            }
            Err(_) => {
                return Self::Invalid("JARVIS_TTS_MODEL_DIR is not valid Unicode.");
            }
        };

        let python = match env::var("JARVIS_TTS_PYTHON") {
            Ok(value) if !value.is_empty() => PathBuf::from(value),
            Ok(_) => {
                return Self::Invalid("JARVIS_TTS_PYTHON cannot be empty.");
            }
            Err(env::VarError::NotPresent) => {
                return Self::Invalid("JARVIS_TTS_PYTHON is required when speech is enabled.");
            }
            Err(env::VarError::NotUnicode(_)) => {
                return Self::Invalid("JARVIS_TTS_PYTHON is not valid Unicode.");
            }
        };
        if !python.is_absolute() {
            return Self::Invalid("JARVIS_TTS_PYTHON must be an absolute path.");
        }

        let worker_script = match optional_env("JARVIS_TTS_WORKER") {
            Ok(None) => current_dir.join("scripts/tts/supertonic_worker.py"),
            Ok(Some(value)) if !value.is_empty() => resolve_from(current_dir, PathBuf::from(value)),
            Ok(Some(_)) => {
                return Self::Invalid("JARVIS_TTS_WORKER cannot be empty.");
            }
            Err(_) => {
                return Self::Invalid("JARVIS_TTS_WORKER is not valid Unicode.");
            }
        };

        let threads = match env::var("JARVIS_TTS_THREADS") {
            Ok(value) => match value.parse::<u32>() {
                Ok(threads @ 1..=4) => threads,
                _ => {
                    return Self::Invalid(
                        "JARVIS_TTS_THREADS must be an integer from 1 through 4.",
                    );
                }
            },
            Err(env::VarError::NotPresent) => 2,
            Err(env::VarError::NotUnicode(_)) => {
                return Self::Invalid("JARVIS_TTS_THREADS is not valid Unicode.");
            }
        };

        let audio_device = match optional_env("JARVIS_AUDIO_DEVICE") {
            Ok(None) => None,
            Ok(Some(value)) if !value.is_empty() => Some(value),
            Ok(Some(_)) => {
                return Self::Invalid("JARVIS_AUDIO_DEVICE cannot be empty.");
            }
            Err(_) => {
                return Self::Invalid("JARVIS_AUDIO_DEVICE is not valid Unicode.");
            }
        };

        Self::Configured(TtsConfig {
            python,
            worker_script,
            model_dir: resolve_from(current_dir, model_dir),
            threads,
            audio_device,
        })
    }
}
