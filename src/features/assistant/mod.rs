mod error;
mod note_facts;
mod policy;
mod policy_hook;
mod run_observer;
mod run_report;

use std::{
    future::Future,
    time::{Duration, Instant},
};

use futures_util::StreamExt;
use rig_agent::{
    Agent,
    agent::{AgentRunner, MultiTurnStreamItem, PromptResponse, StreamingResult},
    streaming::StreamedAssistantContent,
};
use rig_core::{
    completion::Document,
    message::{DocumentMediaType, Message, UserContent},
};
use tokio::sync::mpsc;

pub(crate) use error::AssistantError;
pub(crate) use note_facts::FactExtractor;
pub(crate) use run_report::{AssistantRun, AssistantRunError, CallUsage, ModelStage, RunReport};

use policy_hook::JarvisPolicyHook;
use run_observer::RunObserver;
use run_report::RunOutcome;

/// Jarvis's system prompt. Kept as text so it can be edited without touching code.
pub(crate) const SYSTEM_PREAMBLE: &str = include_str!("preamble.txt");

const RUN_DEADLINE: Duration = Duration::from_secs(30);
const MAX_TEXT_DELTA_BYTES: usize = 1024;
const MAX_RESPONSE_TOKENS: u64 = 1024;
/// How long a cache warm-up may take. A warm-up keeps nobody waiting, and a cold
/// read of the request's start (system prompt, tools and facts, about 1,600
/// tokens at about 58 a second) is close to 30 s on its own (fast-memory 13).
const WARMUP_DEADLINE: Duration = Duration::from_secs(60);
/// A warm-up needs the model server to read the prompt, not to answer it.
const WARMUP_RESPONSE_TOKENS: u64 = 1;

/// The start of the next request, sent ahead of it so the model server reads it
/// while nobody is waiting.
///
/// The server reuses a cached prompt only up to the first token that differs, so
/// after the history is trimmed the next request would be read from the system
/// prompt on. `prompt` is the stored prompt of the last exchange and `history` is
/// everything before it: that is exactly how the next request begins.
///
/// The facts message (see `request_history`) is part of that start, so a warm-up
/// carries the facts the session sent: without them it would warm another prefix.
pub(crate) struct CacheWarmup {
    facts: Option<Message>,
    history: Vec<Message>,
    prompt: Message,
}

impl CacheWarmup {
    pub(crate) fn new(history: Vec<Message>, prompt: Message) -> Self {
        Self {
            facts: None,
            history,
            prompt,
        }
    }

    /// Adds the facts message the next request will carry, `None` for no facts.
    pub(crate) fn with_facts(mut self, facts: Option<Message>) -> Self {
        self.facts = facts;
        self
    }
}

/// One bounded piece of visible assistant text from the native Rig stream.
pub(crate) struct AssistantTextDelta {
    pub(crate) text: String,
}

/// Whether a run may call a tool. Either way every tool is listed, so the request
/// starts as every other does and the server's prompt cache holds (fast-memory 10).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ToolUse {
    /// A prompt from Darren: the model may call one tool on its first turn.
    Offered,
    /// A prompt from Darren that asks about timers: the first turn must call a
    /// tool. Gemma often said "I have set a timer" without calling `set_timer`
    /// (seen on the Pi on 2026-11-26). It forces a call, never which tool, and
    /// grants nothing `Offered` does not.
    Required,
    /// A run Jarvis starts on its own, such as a timer's reminder (timers D20):
    /// the answer is text only, so nothing in it can make Jarvis act.
    Forbidden,
}

// LEARNING: `Assistant` keeps a reusable Rig `Agent`; each `respond` call
// configures a fresh runner for one bounded conversation turn.
pub(crate) struct Assistant {
    agent: Agent,
}

impl Assistant {
    pub(crate) fn new(agent: Agent) -> Self {
        Self { agent }
    }

    /// Run Rig's native multi-turn stream and forward only visible text.
    ///
    /// `facts` is the facts message (`None` for an empty list). It starts the
    /// request, directly after the system prompt. `earlier_history` and
    /// `latest_exchange` are the stored conversation split before its last whole
    /// exchange; recalled `documents` are sent between them (see `request_history`).
    ///
    /// The caller owns both this future and the bounded receiver, so dropping
    /// either side cannot leave a detached producer running in the background.
    #[expect(
        clippy::too_many_arguments,
        reason = "one request's prompt, tool rule, facts, history parts and recall"
    )]
    pub(crate) async fn respond_stream(
        &self,
        prompt: &str,
        tools: ToolUse,
        facts: Option<&Message>,
        earlier_history: &[Message],
        latest_exchange: &[Message],
        documents: &[Document],
        deltas: mpsc::Sender<AssistantTextDelta>,
    ) -> Result<AssistantRun, Box<AssistantRunError>> {
        let observer = RunObserver::default();
        let started_at = Instant::now();

        // LEARNING: the deadline wraps both stream setup and every `next()`;
        // otherwise a provider can keep the foreground run alive forever after
        // returning the initial stream handle.
        let result = tokio::time::timeout(RUN_DEADLINE, async {
            let stream = self
                .configured_runner(
                    prompt,
                    tools,
                    facts,
                    earlier_history,
                    latest_exchange,
                    documents,
                    &observer,
                )
                .stream()
                .await;
            consume_stream(stream, deltas).await
        })
        .await;

        match result {
            Ok(Ok(response)) => finalize_response(response, observer, started_at.elapsed()),
            Ok(Err(error)) => Err(Box::new(AssistantRunError {
                error,
                report: observer.finish(RunOutcome::PromptFailed, started_at.elapsed()),
            })),
            Err(_) => Err(Box::new(AssistantRunError {
                error: AssistantError::RunTimeout,
                report: observer.finish(RunOutcome::TimedOut, started_at.elapsed()),
            })),
        }
    }

    /// Sends `warmup` to the model server and discards whatever comes back.
    ///
    /// Nothing is reported, archived or added to any history. The returned future
    /// borrows nothing, so it can run beside the conversation; dropping it only
    /// stops Jarvis waiting, and whatever the server had read by then stays cached.
    /// It carries the same system prompt, tools and facts message as a real run,
    /// because all three are part of the cached prompt.
    pub(crate) fn warm_cache(
        &self,
        warmup: CacheWarmup,
    ) -> impl Future<Output = Result<(), AssistantError>> + Send + 'static {
        let runner = self
            .agent
            .runner(warmup.prompt)
            .history(request_history(
                warmup.facts.as_ref(),
                &warmup.history,
                &[],
                &[],
            ))
            .max_turns(1)
            .tool_concurrency(1)
            .max_tokens(WARMUP_RESPONSE_TOKENS)
            .max_invalid_tool_call_retries(0)
            .without_memory();
        async move {
            let read = async {
                let mut stream = runner.stream().await;
                while let Some(item) = stream.next().await {
                    item.map_err(AssistantError::Stream)?;
                }
                Ok(())
            };
            match tokio::time::timeout(WARMUP_DEADLINE, read).await {
                Ok(result) => result,
                Err(_) => Err(AssistantError::RunTimeout),
            }
        }
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "one request's prompt, tool rule, facts, history parts and recall"
    )]
    fn configured_runner(
        &self,
        prompt: &str,
        tools: ToolUse,
        facts: Option<&Message>,
        earlier_history: &[Message],
        latest_exchange: &[Message],
        documents: &[Document],
        observer: &RunObserver,
    ) -> AgentRunner {
        // LEARNING: Rig stops this hook chain at the first non-Continue model
        // turn decision. Put the passive observer first so it records the
        // completed model call even when Jarvis policy rejects that turn.
        // Rig's `run.messages()` (and so `PromptResponse::messages`) holds only
        // the prompt and the turns after it, never the input history. The facts and
        // recall messages ride inside that input history, so they are structurally
        // absent from the batch the session replays, without inspecting text.
        let request_history = request_history(facts, earlier_history, documents, latest_exchange);
        self.agent
            .runner(prompt)
            .history(request_history)
            .add_hook(observer.clone())
            .add_hook(JarvisPolicyHook { tools })
            .max_turns(2)
            .tool_concurrency(1)
            .max_tokens(MAX_RESPONSE_TOKENS)
            .max_invalid_tool_call_retries(0)
            .without_memory()
    }
}

/// Returns the input history of a request: `[facts, earlier history, memories,
/// latest exchange]`, and Rig then appends the prompt. A real run and a cache
/// warm-up both build their request through this function, so the warm-up reads
/// exactly the start the next request has.
///
/// - The facts message comes first, directly after the system prompt. It changes
///   only when a fact does, so it stays in the model server's cache. With no facts
///   no message is added.
/// - The memories are one user message placed between the earlier conversation
///   and its last whole exchange. llama.cpp reuses a cached prefix only up to the
///   first token that differs. Recall changes on almost every turn, so Rig's
///   `.documents()` (which inserts right after the system message) made the whole
///   history miss the cache. Here the system prompt, the facts and the earlier
///   history stay byte-identical.
/// - The latest exchange stays next to the question, so a reference such as
///   "there" resolves to the conversation. With the memories directly before the
///   prompt, Gemma copied an old recalled answer instead.
///
/// With no history the memories go directly before the prompt, and with no
/// documents no message is added. The memories message is built exactly as Rig
/// builds it for `.documents()`, so the wire text is unchanged.
fn request_history(
    facts: Option<&Message>,
    earlier_history: &[Message],
    documents: &[Document],
    latest_exchange: &[Message],
) -> Vec<Message> {
    let mut messages = Vec::new();
    messages.extend(facts.cloned());
    messages.extend_from_slice(earlier_history);
    if !documents.is_empty() {
        let content = documents
            .iter()
            .map(|document| {
                UserContent::document(document.to_string(), Some(DocumentMediaType::TXT))
            })
            .collect();
        messages.push(Message::User { content });
    }
    messages.extend_from_slice(latest_exchange);
    messages
}

async fn consume_stream(
    mut stream: StreamingResult,
    deltas: mpsc::Sender<AssistantTextDelta>,
) -> Result<PromptResponse, AssistantError> {
    let mut final_response = None;

    while let Some(item) = stream.next().await {
        match item {
            Ok(MultiTurnStreamItem::StreamAssistantItem(StreamedAssistantContent::Text(text))) => {
                send_text_deltas(&deltas, &text.text).await?
            }
            Ok(MultiTurnStreamItem::FinalResponse(response)) => {
                final_response = Some(response);
            }
            Ok(_) => {
                // Reasoning, tool-call JSON, tool results, retry metadata, and
                // provider metadata are structured events, not visible text.
            }
            Err(error) => return Err(AssistantError::Stream(error)),
        }
    }

    final_response.ok_or(AssistantError::MissingFinalResponse)
}

async fn send_text_deltas(
    deltas: &mpsc::Sender<AssistantTextDelta>,
    mut text: &str,
) -> Result<(), AssistantError> {
    while !text.is_empty() {
        // LEARNING: a bounded channel limits the number of queued items. This
        // byte cap also limits memory when one provider chunk is unusually large;
        // backing up to a UTF-8 boundary keeps every emitted String valid.
        let mut end = text.len().min(MAX_TEXT_DELTA_BYTES);
        while !text.is_char_boundary(end) {
            end -= 1;
        }

        let chunk = text[..end].to_owned();
        deltas
            .send(AssistantTextDelta { text: chunk })
            .await
            .map_err(|_| AssistantError::DeltaReceiverClosed)?;
        text = &text[end..];
    }

    Ok(())
}

// LEARNING: `Box<AssistantRunError>` owns the typed error value on the heap;
// this is not Java's primitive boxing. The error still carries its run report.
fn finalize_response(
    response: PromptResponse,
    observer: RunObserver,
    elapsed: Duration,
) -> Result<AssistantRun, Box<AssistantRunError>> {
    let invalid_response = response.output.trim().is_empty();
    let outcome = if invalid_response {
        RunOutcome::InvalidResponse
    } else {
        RunOutcome::Success
    };
    let report = observer.finish(outcome, elapsed);

    if invalid_response {
        Err(Box::new(AssistantRunError {
            error: AssistantError::InvalidResponse,
            report,
        }))
    } else {
        Ok(AssistantRun { response, report })
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/features/assistant/assistant_tests.rs"]
mod tests;
