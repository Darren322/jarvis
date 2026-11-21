//! The extraction run behind noting facts while idle (fast-memory 05): one bounded
//! model run that reads something Darren said and answers with the facts in it, as
//! text for the caller to parse. The answer is untrusted; nothing here stores it.
//!
//! This is its own agent, not a second use of the conversation's: no tools are
//! registered (so the request lists none and the model cannot call one), the
//! policy hook that offers tools is not attached, and the system prompt, the facts
//! list and the conversation are not sent. The request is the fixed prompt in
//! `note_facts.txt` with the turn's text at its very end, so everything before that
//! text is identical on every run and the model server keeps it in its cache.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use futures_util::StreamExt;
use rig_agent::{
    Agent, AgentBuilder,
    agent::{
        MultiTurnStreamItem,
        hook::{AgentHook, HookContext, ModelTurnAction, ModelTurnFinished},
    },
    prelude::CompletionModel,
    streaming::StreamedAssistantContent,
};
use rig_core::completion::FinishReason;

use super::AssistantError;

/// The extraction prompt, as text so it can be read and edited without touching
/// code. `{said}` marks where the turn's text goes.
pub(crate) const NOTE_FACTS_PROMPT: &str = include_str!("note_facts.txt");
const SAID_MARKER: &str = "{said}";

/// How long one extraction may take, from the request to the last word. Reading the
/// prompt (about 250 tokens, cached after the first run) and writing a few short
/// lines is a few seconds at the model server's measured speed; the HTTP client's
/// own 30 s limit is the same ceiling.
const NOTE_FACTS_DEADLINE: Duration = Duration::from_secs(30);
/// Five short facts fit well inside this; a model that rambles is cut off here.
const NOTE_FACTS_MAX_TOKENS: u64 = 160;
/// An extraction should give the same answer for the same words.
const NOTE_FACTS_TEMPERATURE: f64 = 0.0;
/// Text past this is not read: the run's answer is a handful of short lines.
const MAX_NOTE_FACTS_OUTPUT_BYTES: usize = 2_048;

/// Runs the extraction prompt against the model server.
pub(crate) struct FactExtractor {
    agent: Agent,
}

impl FactExtractor {
    /// An agent with no tools and no system prompt of its own: the prompt is the
    /// request.
    pub(crate) fn new<M>(model: M) -> Self
    where
        M: CompletionModel + 'static,
    {
        Self {
            agent: AgentBuilder::new(model)
                .temperature(NOTE_FACTS_TEMPERATURE)
                .build(),
        }
    }

    /// Asks the model for the facts in `said` (Darren's words, nothing else) and
    /// returns its raw answer. An answer the provider reports as cut short (token
    /// limit, filter) returns no text at all: a cut-off last line could be a wrong
    /// fact.
    ///
    /// The deadline covers the model only. Safe to drop at any `.await`: nothing is
    /// stored or held, and dropping the stream stops Jarvis waiting for the server.
    pub(crate) async fn extract(&self, said: &str) -> Result<String, AssistantError> {
        let cut_off = CutOffSpy::default();
        let runner = self
            .agent
            .runner(render_prompt(said))
            .add_hook(cut_off.clone())
            .max_turns(1)
            .tool_concurrency(1)
            .max_tokens(NOTE_FACTS_MAX_TOKENS)
            .max_invalid_tool_call_retries(0)
            .without_memory();
        let read = async {
            let mut stream = runner.stream().await;
            let mut text = String::new();
            let mut finished = false;
            while let Some(item) = stream.next().await {
                match item.map_err(AssistantError::Stream)? {
                    MultiTurnStreamItem::StreamAssistantItem(StreamedAssistantContent::Text(
                        delta,
                    )) => {
                        text.push_str(&delta.text);
                        if text.len() > MAX_NOTE_FACTS_OUTPUT_BYTES {
                            return Ok(String::new());
                        }
                    }
                    MultiTurnStreamItem::FinalResponse(_) => finished = true,
                    // Reasoning, usage and metadata are not part of the answer.
                    _ => {}
                }
            }
            if finished {
                Ok(text)
            } else {
                Err(AssistantError::MissingFinalResponse)
            }
        };
        let text = match tokio::time::timeout(NOTE_FACTS_DEADLINE, read).await {
            Ok(result) => result?,
            Err(_) => return Err(AssistantError::RunTimeout),
        };
        Ok(if cut_off.was_cut_off() {
            String::new()
        } else {
            text
        })
    }
}

/// The prompt with `said` where the marker is. A prompt file without the marker
/// still ends with the text, so an edit cannot make the run panic.
fn render_prompt(said: &str) -> String {
    match NOTE_FACTS_PROMPT.split_once(SAID_MARKER) {
        Some((head, tail)) => format!("{head}{said}{tail}"),
        None => format!("{NOTE_FACTS_PROMPT}\n{said}"),
    }
}

/// Records whether the provider said it stopped for any reason but finishing.
#[derive(Clone, Default)]
struct CutOffSpy(Arc<AtomicBool>);

impl CutOffSpy {
    fn was_cut_off(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

impl AgentHook for CutOffSpy {
    async fn on_model_turn_finished(
        &self,
        _ctx: &HookContext,
        event: ModelTurnFinished<'_>,
    ) -> ModelTurnAction {
        // No reason at all is common on OpenAI-compatible servers and is not a cut.
        if event
            .finish_reason
            .is_some_and(|reason| !matches!(reason, FinishReason::Stop))
        {
            self.0.store(true, Ordering::Release);
        }
        ModelTurnAction::Continue
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/features/assistant/note_facts_tests.rs"]
mod tests;
