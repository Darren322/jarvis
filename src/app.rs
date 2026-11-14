use std::path::PathBuf;

use rig_agent::AgentBuilder;
use tokio::{
    io::BufReader,
    sync::{mpsc, watch},
};

use crate::clients::{
    local_llm::LocalLlm, remote_embeddings::RemoteEmbeddings, speech_to_text::SpeechToText,
};
use crate::config::{AppConfig, OptionalTtsConfig, OrbConfig};
use crate::features::{
    assistant::{Assistant, SYSTEM_PREAMBLE},
    conversation::ConversationSession,
    listening::{Listener, Recorder, run_level_meter},
    memory::MemoryService,
    speech::SpeechOutput,
    state::StateFeed,
};
use crate::interfaces::cli::{
    diagnostics::error_chain,
    input::Taps,
    presentation::{StatePublishing, TerminalPresenter},
    repl::run_input_loop,
};
use crate::interfaces::orb::{OrbFeeds, OrbServer, TAP_QUEUE};
use crate::tools;

/// The ALSA recording program; tests substitute a fake script.
const CAPTURE_PROGRAM: &str = "arecord";

// LEARNING: `App` is like a Java class's related fields; its `impl` block below
// supplies methods, while Rust declares data and behavior in separate blocks.
pub struct App {
    assistant: Assistant,
    local_llm: LocalLlm,
    embeddings: RemoteEmbeddings,
    archive_path: PathBuf,
    tts_config: OptionalTtsConfig,
    /// `None` when the speech-to-text settings are absent: `/listen` says so.
    listener: Option<Listener>,
    orb_config: OrbConfig,
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
        let listener = match &config.speech_to_text {
            Some(stt) => Some(Listener::new(
                Recorder::new(&stt.capture_device, CAPTURE_PROGRAM),
                SpeechToText::new(stt)?,
            )),
            None => None,
        };

        // LEARNING: `Self` means `App` here. Field shorthand moves each local
        // into its same-named field; it does not make an extra copy.
        Ok(Self {
            assistant,
            local_llm,
            embeddings,
            archive_path: config.archive_path.clone(),
            tts_config: config.tts.clone(),
            listener,
            orb_config: config.orb.clone(),
        })
    }

    pub async fn run(&self) -> Result<(), Box<dyn std::error::Error>> {
        self.local_llm.health_check().await?;
        self.warn_if_speech_to_text_is_down().await;

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

        // The orb screen is a second front end beside the terminal. The two meet only
        // here: the terminal publishes its states and reads taps, the orb interface
        // shows the states and loudness and sends taps.
        let states = StateFeed::new();
        let (mic_levels, mic_level) = watch::channel(0.0);
        let (tap_sender, tap_receiver) = mpsc::channel(TAP_QUEUE);
        let orb = self.open_orb();
        let taps = match orb {
            Some(_) => Taps::new(tap_receiver),
            None => Taps::none(),
        };
        let feeds = OrbFeeds {
            states: &states,
            mic_level,
            speech_level: speech.as_ref().map(SpeechOutput::level),
            taps: tap_sender,
        };
        let serve_orb = async {
            let Some(orb) = orb else {
                return std::future::pending().await;
            };
            let meter = async {
                if let Some(listener) = &self.listener {
                    run_level_meter(listener.tap(), mic_levels).await;
                }
            };
            // `serve` never returns, so neither does the pair.
            tokio::join!(orb.serve(feeds), meter).0
        };

        let stdin = tokio::io::stdin();
        let mut input = BufReader::new(stdin);
        let mut presenter = StatePublishing::new(TerminalPresenter::default(), &states);
        // The orb runs beside the REPL in this task and ends with it: dropping
        // `serve_orb` closes its connections and removes the socket file.
        tokio::select! {
            result = run_input_loop(
                &self.assistant,
                &mut session,
                memory.as_ref(),
                &mut speech,
                &mut input,
                taps,
                &mut presenter,
                self.listener.as_ref(),
            ) => result,
            never = serve_orb => match never {},
        }
    }

    /// Opens the orb's socket. The orb is optional, so a socket that cannot be
    /// opened is a warning and Jarvis runs without it.
    fn open_orb(&self) -> Option<OrbServer> {
        match OrbServer::bind(&self.orb_config.socket_path) {
            Ok(orb) => Some(orb),
            Err(error) => {
                eprintln!(
                    "Warning: the orb screen is unavailable: {}",
                    error_chain(&error)
                );
                None
            }
        }
    }

    /// The server may come up later, so a failed probe warns and leaves `/listen` on.
    async fn warn_if_speech_to_text_is_down(&self) {
        if let Some(listener) = &self.listener
            && let Err(error) = listener.health_check().await
        {
            eprintln!(
                "Warning: the speech-to-text server is not reachable; /listen will try again when used: {}",
                error_chain(&error)
            );
        }
    }
}
