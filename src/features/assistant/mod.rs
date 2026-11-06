mod error;
mod policy;
mod policy_hook;
mod run_observer;
mod run_report;

use std::time::{Duration, Instant};

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
pub(crate) use run_report::{AssistantRun, AssistantRunError, CallUsage, ModelStage, RunReport};

use policy_hook::JarvisPolicyHook;
use run_observer::RunObserver;
use run_report::RunOutcome;

/// Jarvis's system prompt. Kept as text so it can be edited without touching code.
pub(crate) const SYSTEM_PREAMBLE: &str = include_str!("preamble.txt");

const RUN_DEADLINE: Duration = Duration::from_secs(30);
const MAX_TEXT_DELTA_BYTES: usize = 1024;
const MAX_RESPONSE_TOKENS: u64 = 1024;

/// One bounded piece of visible assistant text from the native Rig stream.
pub(crate) struct AssistantTextDelta {
    pub(crate) text: String,
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
    /// `earlier_history` and `latest_exchange` are the stored conversation
    /// split before its last whole exchange; recalled `documents` are sent
    /// between them (see `history_with_recall`).
    ///
    /// The caller owns both this future and the bounded receiver, so dropping
    /// either side cannot leave a detached producer running in the background.
    pub(crate) async fn respond_stream(
        &self,
        prompt: &str,
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

    fn configured_runner(
        &self,
        prompt: &str,
        earlier_history: &[Message],
        latest_exchange: &[Message],
        documents: &[Document],
        observer: &RunObserver,
    ) -> AgentRunner {
        // LEARNING: Rig stops this hook chain at the first non-Continue model
        // turn decision. Put the passive observer first so it records the
        // completed model call even when Jarvis policy rejects that turn.
        // Rig's `run.messages()` (and so `PromptResponse::messages`) holds only
        // the prompt and the turns after it, never the input history. The recall
        // message rides inside that input history, so it is structurally absent
        // from the batch the session replays, without inspecting text.
        let request_history = history_with_recall(earlier_history, latest_exchange, documents);
        self.agent
            .runner(prompt)
            .history(request_history)
            .add_hook(observer.clone())
            .add_hook(JarvisPolicyHook)
            .max_turns(2)
            .tool_concurrency(1)
            .max_tokens(MAX_RESPONSE_TOKENS)
            .max_invalid_tool_call_retries(0)
            .without_memory()
    }
}

/// Returns the input history with the recalled memories as one user message
/// placed between the earlier conversation and its last whole exchange.
///
/// The request order is `[earlier history, memories, latest exchange]`, and
/// Rig then appends the prompt. The placement serves two needs:
///
/// - llama.cpp reuses a cached prefix only up to the first token that differs.
///   Recall changes on almost every turn, so Rig's `.documents()` (which inserts
///   right after the system message) made the whole history miss the cache.
///   Here the system prompt and the earlier history stay byte-identical.
/// - The latest exchange stays next to the question, so a reference such as
///   "there" resolves to the conversation. With the memories directly before
///   the prompt, Gemma copied an old recalled answer instead.
///
/// With no history the memories go directly before the prompt, and with no
/// documents no message is added. The message is built exactly as Rig builds it
/// for `.documents()`, so the wire text is unchanged.
fn history_with_recall(
    earlier_history: &[Message],
    latest_exchange: &[Message],
    documents: &[Document],
) -> Vec<Message> {
    let mut messages = earlier_history.to_vec();
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
