use std::env;

use super::{ConfigError, optional_env};

const BASE_URL_VAR: &str = "JARVIS_STT_BASE_URL";
const MODEL_VAR: &str = "JARVIS_STT_MODEL";
const CAPTURE_DEVICE_VAR: &str = "JARVIS_CAPTURE_DEVICE";

/// Speech-to-text and the microphone it listens through. Optional as a whole:
/// see [`SpeechToTextConfig::from_lookup`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpeechToTextConfig {
    /// OpenAI-style base URL, such as `http://jarvis-ai.local:8082/v1`.
    pub base_url: String,
    pub model: String,
    /// An ALSA device name such as `plughw:3,0`, not a file path.
    pub capture_device: String,
}

impl SpeechToTextConfig {
    pub(super) fn from_env() -> Result<Option<Self>, ConfigError> {
        Self::from_lookup(optional_env)
    }

    /// Reads settings through `lookup` so the rules can be tested without touching the
    /// process environment.
    ///
    /// `Ok(None)` means none of the three settings is present, so Jarvis starts without
    /// voice input. Setting only some of them is a mistake and names the first missing
    /// one. An empty value counts as unset.
    fn from_lookup(
        lookup: impl Fn(&'static str) -> Result<Option<String>, env::VarError>,
    ) -> Result<Option<Self>, ConfigError> {
        let read = |name: &'static str| -> Result<Option<String>, ConfigError> {
            let value = lookup(name).map_err(|source| ConfigError::Var { name, source })?;
            Ok(value.filter(|value| !value.is_empty()))
        };
        let base_url = read(BASE_URL_VAR)?;
        let model = read(MODEL_VAR)?;
        let capture_device = read(CAPTURE_DEVICE_VAR)?;

        match (base_url, model, capture_device) {
            (None, None, None) => Ok(None),
            (Some(base_url), Some(model), Some(capture_device)) => Ok(Some(Self {
                base_url,
                model,
                capture_device,
            })),
            (None, ..) => Err(ConfigError::SpeechToTextIncomplete { name: BASE_URL_VAR }),
            (_, None, _) => Err(ConfigError::SpeechToTextIncomplete { name: MODEL_VAR }),
            (.., None) => Err(ConfigError::SpeechToTextIncomplete {
                name: CAPTURE_DEVICE_VAR,
            }),
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/config/speech_to_text_tests.rs"]
mod tests;
