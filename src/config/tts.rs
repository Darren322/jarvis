use std::{
    env,
    path::{Path, PathBuf},
};

use thiserror::Error;

use super::{optional_env, resolve_from};

const ENGINE_VAR: &str = "JARVIS_TTS_ENGINE";
const MODEL_DIR_VAR: &str = "JARVIS_TTS_MODEL_DIR";
const PIPER_MODEL_VAR: &str = "JARVIS_TTS_PIPER_MODEL";
const PYTHON_VAR: &str = "JARVIS_TTS_PYTHON";
const WORKER_VAR: &str = "JARVIS_TTS_WORKER";
const THREADS_VAR: &str = "JARVIS_TTS_THREADS";
const AUDIO_DEVICE_VAR: &str = "JARVIS_AUDIO_DEVICE";

const SUPERTONIC_WORKER_SCRIPT: &str = "scripts/tts/supertonic_worker.py";
const PIPER_WORKER_SCRIPT: &str = "scripts/tts/piper_worker.py";
const PIPER_MODEL_EXTENSION: &str = ".onnx";
const DEFAULT_THREADS: u32 = 2;
const MAX_THREADS: u32 = 4;

#[derive(Clone, Debug)]
pub enum OptionalTtsConfig {
    Disabled,
    Invalid(TtsConfigError),
    Configured(TtsConfig),
}

/// Why speech settings were rejected. Speech is optional, so this disables speech and
/// never stops startup.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum TtsConfigError {
    #[error("{name} is not valid Unicode")]
    NotUnicode { name: &'static str },
    #[error("{name} cannot be empty")]
    Empty { name: &'static str },
    #[error("{name} is required when speech is enabled")]
    Missing { name: &'static str },
    #[error("{name} must be an absolute path")]
    NotAbsolute { name: &'static str },
    #[error("{THREADS_VAR} must be an integer from 1 through {MAX_THREADS}")]
    Threads,
    #[error("{ENGINE_VAR} must be supertonic or piper, not {value:?}")]
    UnknownEngine { value: String },
    #[error("{PIPER_MODEL_VAR} must name a model file such as voice{PIPER_MODEL_EXTENSION}")]
    PiperModelNotAFile,
}

#[derive(Clone, Debug)]
pub struct TtsConfig {
    pub python: PathBuf,
    pub worker_script: PathBuf,
    pub engine: TtsEngine,
    pub threads: u32,
    pub audio_device: Option<String>,
}

/// The speech engine and the voice it loads. Each engine owns exactly the settings it needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TtsEngine {
    /// Supertonic 3 INT8 in a prepared directory that holds the model and `voice.bin`.
    Supertonic { model_dir: PathBuf },
    /// One Piper voice. `voice` is the model file name without `.onnx`, which is the
    /// name the worker reports when it is ready.
    Piper { model: PathBuf, voice: String },
}

/// The engine setting before its paths are read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EngineChoice {
    Supertonic,
    Piper,
}

impl EngineChoice {
    /// An unset engine is Supertonic, so `.env` files from before Piper keep working.
    fn parse(value: Option<&str>) -> Result<Self, TtsConfigError> {
        match value {
            None | Some("supertonic") => Ok(Self::Supertonic),
            Some("piper") => Ok(Self::Piper),
            Some("") => Err(TtsConfigError::Empty { name: ENGINE_VAR }),
            Some(other) => Err(TtsConfigError::UnknownEngine {
                value: other.to_owned(),
            }),
        }
    }

    fn default_worker_script(self) -> &'static str {
        match self {
            Self::Supertonic => SUPERTONIC_WORKER_SCRIPT,
            Self::Piper => PIPER_WORKER_SCRIPT,
        }
    }
}

impl OptionalTtsConfig {
    pub(super) fn from_env(current_dir: &Path) -> Self {
        Self::from_lookup(current_dir, optional_env)
    }

    /// Reads settings through `lookup` so the rules can be tested without touching the
    /// process environment.
    fn from_lookup(
        current_dir: &Path,
        lookup: impl Fn(&'static str) -> Result<Option<String>, env::VarError>,
    ) -> Self {
        match TtsConfig::parse(current_dir, lookup) {
            Ok(Some(config)) => Self::Configured(config),
            Ok(None) => Self::Disabled,
            Err(error) => Self::Invalid(error),
        }
    }
}

impl TtsConfig {
    /// `Ok(None)` means speech is off: the engine's model setting is absent and was
    /// never requested.
    fn parse(
        current_dir: &Path,
        lookup: impl Fn(&'static str) -> Result<Option<String>, env::VarError>,
    ) -> Result<Option<Self>, TtsConfigError> {
        let text = |name: &'static str| -> Result<Option<String>, TtsConfigError> {
            lookup(name).map_err(|_| TtsConfigError::NotUnicode { name })
        };
        let non_empty = |name: &'static str| -> Result<Option<String>, TtsConfigError> {
            match text(name)? {
                Some(value) if value.is_empty() => Err(TtsConfigError::Empty { name }),
                value => Ok(value),
            }
        };

        let choice = EngineChoice::parse(text(ENGINE_VAR)?.as_deref())?;
        let engine = match choice {
            EngineChoice::Supertonic => match non_empty(MODEL_DIR_VAR)? {
                None => return Ok(None),
                Some(model_dir) => TtsEngine::Supertonic {
                    model_dir: resolve_from(current_dir, PathBuf::from(model_dir)),
                },
            },
            EngineChoice::Piper => {
                let model = non_empty(PIPER_MODEL_VAR)?.ok_or(TtsConfigError::Missing {
                    name: PIPER_MODEL_VAR,
                })?;
                let model = resolve_from(current_dir, PathBuf::from(model));
                let voice = piper_voice_name(&model)?;
                TtsEngine::Piper { model, voice }
            }
        };

        let python = non_empty(PYTHON_VAR)?
            .map(PathBuf::from)
            .ok_or(TtsConfigError::Missing { name: PYTHON_VAR })?;
        if !python.is_absolute() {
            return Err(TtsConfigError::NotAbsolute { name: PYTHON_VAR });
        }

        let worker_script = match non_empty(WORKER_VAR)? {
            Some(value) => resolve_from(current_dir, PathBuf::from(value)),
            None => current_dir.join(choice.default_worker_script()),
        };

        let threads = match text(THREADS_VAR)? {
            None => DEFAULT_THREADS,
            Some(value) => match value.parse::<u32>() {
                Ok(threads @ 1..=MAX_THREADS) => threads,
                _ => return Err(TtsConfigError::Threads),
            },
        };

        Ok(Some(Self {
            python,
            worker_script,
            engine,
            threads,
            audio_device: non_empty(AUDIO_DEVICE_VAR)?,
        }))
    }
}

fn piper_voice_name(model: &Path) -> Result<String, TtsConfigError> {
    let file_name = model
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(TtsConfigError::PiperModelNotAFile)?;
    let voice = file_name
        .strip_suffix(PIPER_MODEL_EXTENSION)
        .unwrap_or(file_name);
    if voice.is_empty() {
        return Err(TtsConfigError::PiperModelNotAFile);
    }
    Ok(voice.to_owned())
}

#[cfg(test)]
#[path = "../../tests/unit/config/tts_tests.rs"]
mod tests;
