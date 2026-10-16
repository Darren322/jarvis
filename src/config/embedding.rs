use super::{ConfigError, required_env};

/// The BGE embedding server on the LAN (`llama-server --embedding`).
pub struct EmbeddingConfig {
    /// OpenAI-compatible base URL, such as `http://jarvis-ai.local:8081/v1`.
    pub base_url: String,
    pub model: String,
}

impl EmbeddingConfig {
    pub(super) fn from_env() -> Result<Self, ConfigError> {
        Ok(Self {
            base_url: required_env("JARVIS_EMBEDDING_BASE_URL")?,
            model: required_env("JARVIS_EMBEDDING_MODEL")?,
        })
    }
}
