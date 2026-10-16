use std::path::PathBuf;

use rig_agent::AgentBuilder;
use tokio::io::BufReader;

use crate::clients::{local_llm::LocalLlm, remote_embeddings::RemoteEmbeddings};
use crate::config::{AppConfig, OptionalTtsConfig};
use crate::features::{
    assistant::{Assistant, SYSTEM_PREAMBLE},
    conversation::ConversationSession,
    memory::MemoryService,
    speech::SpeechOutput,
};
use crate::interfaces::cli::{presentation::TerminalPresenter, repl::run_input_loop};
use crate::tools;

// LEARNING: `App` is like a Java class's related fields; its `impl` block below
// supplies methods, while Rust declares data and behavior in separate blocks.
pub struct App {
    assistant: Assistant,
    local_llm: LocalLlm,
    embeddings: RemoteEmbeddings,
    archive_path: PathBuf,
    tts_config: OptionalTtsConfig,
}

impl App {
    // LEARNING: `App::new` is an associated constructor (a naming convention,
    // not a special Rust constructor). `&AppConfig` borrows settings so this
    // call need not take ownership of config values it only reads.
    pub fn new(config: &AppConfig) -> Result<Self, Box<dyn std::error::Error>> {
        let local_llm = LocalLlm::new(&config.local_llm)?;
        let embeddings = RemoteEmbeddings::new(&config.embedding)?;
        // LEARNING: Java fluent builders often mutate and return `this`; these
        // Rust methods take and return the owned builder. `.build()` makes a
        // reusable `Agent` but sends no request. The preamble guides the model;
        // policy hooks enforce rules. `tools::register` passes the builder
        // through every `.tool(...)` call, registering each tool's schema and
        // implementation for Rig dispatch.
        let agent =
            tools::register(AgentBuilder::new(local_llm.model()).preamble(SYSTEM_PREAMBLE)).build();
        let assistant = Assistant::new(agent);

        // LEARNING: `Self` means `App` here. Field shorthand moves each local
        // into its same-named field; it does not make an extra copy.
        Ok(Self {
            assistant,
            local_llm,
            embeddings,
            archive_path: config.archive_path.clone(),
            tts_config: config.tts.clone(),
        })
    }

    pub async fn run(&self) -> Result<(), Box<dyn std::error::Error>> {
        self.local_llm.health_check().await?;

        let mut speech = match &self.tts_config {
            OptionalTtsConfig::Disabled => None,
            OptionalTtsConfig::Invalid(reason) => {
                eprintln!("Warning: local speech is disabled: {reason}");
                None
            }
            OptionalTtsConfig::Configured(config) => Some(SpeechOutput::new(config.clone())),
        };

        println!("Conversation archive: {}", self.archive_path.display());
        let mut session = ConversationSession::new(self.archive_path.clone());
        if let Err(error) = session.reset().await {
            eprintln!(
                "Warning: local archiving is unavailable; this session will use process-only context: {error}"
            );
        }
        let repository = session.memory_repository();
        let memory = match repository {
            Some(repository) => Some(MemoryService::open(repository, self.embeddings.clone())),
            None => {
                eprintln!(
                    "Warning: local memory storage is unavailable; continuing with normal chat."
                );
                None
            }
        };
        println!("Enter a message, or /help for commands.");

        let stdin = tokio::io::stdin();
        let mut input = BufReader::new(stdin);
        let mut presenter = TerminalPresenter::default();
        run_input_loop(
            &self.assistant,
            &mut session,
            memory.as_ref(),
            &mut speech,
            &mut input,
            &mut presenter,
        )
        .await
    }
}
