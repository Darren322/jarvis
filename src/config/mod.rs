use std::env;

pub struct AppConfig {
    pub local_llm_base_url: String,
    pub local_llm_health_url: String,
    pub local_llm_model: String,
}

impl AppConfig {
    pub fn load() -> Result<Self, env::VarError> {
        dotenvy::dotenv().ok();

        Ok(Self {
            local_llm_base_url: env::var("LOCAL_LLM_BASE_URL")?,
            local_llm_health_url: env::var("LOCAL_LLM_HEALTH_URL")?,
            local_llm_model: env::var("LOCAL_LLM_MODEL")?,
        })
    }
}
