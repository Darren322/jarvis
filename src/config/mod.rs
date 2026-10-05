use std::env;

const DEFAULT_ARCHIVE_PATH: &str = "data/jarvis.sqlite3";

pub struct AppConfig {
    pub local_llm_base_url: String,
    pub local_llm_health_url: String,
    pub local_llm_model: String,
    pub archive_path: String,
}

impl AppConfig {
    pub fn load() -> Result<Self, env::VarError> {
        dotenvy::dotenv().ok();

        Ok(Self {
            local_llm_base_url: env::var("LOCAL_LLM_BASE_URL")?,
            local_llm_health_url: env::var("LOCAL_LLM_HEALTH_URL")?,
            local_llm_model: env::var("LOCAL_LLM_MODEL")?,
            // LEARNING: `|_| expression` is closure syntax; `_` ignores its
            // `VarError` argument. `unwrap_or_else` runs it only for `Err`.
            archive_path: env::var("JARVIS_ARCHIVE_PATH")
                .unwrap_or_else(|_| DEFAULT_ARCHIVE_PATH.to_string()),
        })
    }
}
