use super::{ConfigError, required_env};

pub struct LocalLlmConfig {
    pub base_url: String,
    pub health_url: String,
    pub model: String,
}

impl LocalLlmConfig {
    pub(super) fn from_env() -> Result<Self, ConfigError> {
        Ok(Self {
            base_url: required_env("LOCAL_LLM_BASE_URL")?,
            health_url: required_env("LOCAL_LLM_HEALTH_URL")?,
            model: required_env("LOCAL_LLM_MODEL")?,
        })
    }
}
