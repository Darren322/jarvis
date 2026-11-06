use futures_util::StreamExt as _;
use rig_agent::{
    AgentBuilder,
    agent::{MultiTurnStreamItem, PromptResponse, StreamingError},
    prelude::{CompletionError, CompletionModel},
    streaming::StreamedAssistantContent,
    tool::Tool,
};
use rig_core::{
    completion::{CompletionRequest, CompletionResponse, Document, Usage},
    message::{AssistantContent, DocumentMediaType, Message, Text, UserContent},
    streaming::StreamingCompletionResponse,
    test_utils::{MockCompletionModel, MockStreamEvent},
};
use serde_json::{Value, json};
use std::{collections::HashMap, time::Duration};
use tokio::sync::{mpsc, oneshot};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

use crate::{
    clients::local_llm::LocalLlm,
    config::LocalLlmConfig,
    features::assistant::{
        Assistant,
        error::AssistantError,
        run_observer::RunObserver,
        run_report::{
            AssistantRun, AssistantRunError, CallUsage, ModelStage, RunOutcome, ToolStageOutcome,
        },
    },
    tools::system_status_tool::SystemStatusTool,
};

use super::{MAX_TEXT_DELTA_BYTES, consume_stream, send_text_deltas};

impl Assistant {
    /// Runs the production streaming path and drains visible deltas, as the
    /// CLI does. Tests that inspect individual deltas call `respond_stream`.
    pub(crate) async fn respond(
        &self,
        prompt: &str,
        history: &[Message],
    ) -> Result<AssistantRun, Box<AssistantRunError>> {
        self.respond_with_recall(prompt, history, &[]).await
    }

    /// Same as `respond`, with recalled memories supplied as the session does.
    async fn respond_with_recall(
        &self,
        prompt: &str,
        history: &[Message],
        documents: &[Document],
    ) -> Result<AssistantRun, Box<AssistantRunError>> {
        let (deltas, mut receiver) = mpsc::channel(16);
        let drain = async { while receiver.recv().await.is_some() {} };
        let (result, ()) = tokio::join!(
            self.respond_stream(prompt, history, documents, deltas),
            drain
        );
        result
    }
}

/// One scripted streamed model turn: visible text, then the final event.
pub(crate) fn streamed_text(text: &str, usage: Usage) -> Vec<MockStreamEvent> {
    vec![
        MockStreamEvent::text(text),
        MockStreamEvent::final_response(usage),
    ]
}

#[derive(Clone)]
struct PendingCompletionModel;

impl CompletionModel for PendingCompletionModel {
    async fn completion(
        &self,
        _request: CompletionRequest,
    ) -> Result<CompletionResponse, CompletionError> {
        // The assistant only streams; this path is never used.
        unreachable!("Assistant uses the streaming path")
    }

    async fn stream(
        &self,
        _request: CompletionRequest,
    ) -> Result<StreamingCompletionResponse, CompletionError> {
        // Simulate a provider that accepts the request but never streams.
        std::future::pending().await
    }
}

fn assert_incomplete_model(stage: &ModelStage, turn: usize) {
    assert_eq!(stage.turn, turn);
    assert!(!stage.completed);
    assert!(stage.elapsed.is_none());
    assert!(matches!(&stage.usage, CallUsage::Unavailable));
}

#[tokio::test]
async fn assistant_text_run_returns_success_report_without_tools() {
    let model =
        MockCompletionModel::from_stream_turns([streamed_text("JARVIS ONLINE", Usage::new())]);
    let agent = AgentBuilder::new(model).tool(SystemStatusTool).build();
    let assistant = Assistant::new(agent);

    let run = assistant
        .respond("Reply with exactly: JARVIS ONLINE", &[])
        .await
        .expect("assistant should return a text response");

    assert_eq!(run.response.output, "JARVIS ONLINE");
    assert!(matches!(run.report.outcome, RunOutcome::Success));
    assert!(run.report.observations_available);
    assert!(run.report.tool_stages.is_empty());
    assert_eq!(run.report.model_stages.len(), 1);
    let stage = &run.report.model_stages[0];
    assert_eq!(stage.turn, 1);
    assert!(stage.completed);
    assert!(stage.elapsed.is_some());
    assert!(matches!(&stage.usage, CallUsage::Unavailable));
}

#[tokio::test]
async fn assistant_reports_usage_per_run_and_marks_missing_usage_unavailable() {
    let expected = Usage {
        input_tokens: 17,
        output_tokens: 9,
        total_tokens: 26,
        cached_input_tokens: 3,
        cache_creation_input_tokens: 4,
        tool_use_prompt_tokens: 5,
        reasoning_tokens: 6,
    };
    let model = MockCompletionModel::from_stream_turns([
        streamed_text("usage reported", expected),
        streamed_text("usage omitted", Usage::new()),
    ]);
    let model_handle = model.clone();
    let assistant = Assistant::new(AgentBuilder::new(model).tool(SystemStatusTool).build());

    let first = assistant
        .respond("first run", &[])
        .await
        .expect("first run succeeds");
    let second = assistant
        .respond("independent second run", &[])
        .await
        .expect("second run succeeds");

    assert_eq!(model_handle.request_count(), 2);
    assert!(first.report.observations_available);
    assert!(second.report.observations_available);
    assert_eq!(first.report.model_stages.len(), 1);
    assert_eq!(second.report.model_stages.len(), 1);
    assert_eq!(first.report.model_stages[0].turn, 1);
    assert_eq!(second.report.model_stages[0].turn, 1);

    match &first.report.model_stages[0].usage {
        CallUsage::Normalized(actual) => assert_eq!(actual, &expected),
        CallUsage::Unavailable => panic!("reported usage must be preserved"),
    }
    assert!(matches!(
        &second.report.model_stages[0].usage,
        CallUsage::Unavailable
    ));
}

#[tokio::test(start_paused = true)]
async fn streaming_run_times_out_while_waiting_for_the_provider_stream() {
    let agent = AgentBuilder::new(PendingCompletionModel)
        .tool(SystemStatusTool)
        .build();
    let assistant = Assistant::new(agent);
    let (delta_tx, _delta_rx) = mpsc::channel(2);

    let error = assistant
        .respond_stream("Hello", &[], &[], delta_tx)
        .await
        .expect_err("pending provider stream should hit the Assistant deadline");

    assert!(matches!(&error.error, AssistantError::RunTimeout));
    assert!(matches!(&error.report.outcome, RunOutcome::TimedOut));
    assert_eq!(error.report.model_stages.len(), 1);
    assert_incomplete_model(&error.report.model_stages[0], 1);
}

#[tokio::test]
async fn provider_prompt_failure_reports_incomplete_model_stage_and_keeps_error_chain() {
    let model =
        MockCompletionModel::from_stream_turns([vec![MockStreamEvent::error("provider failed")]]);
    let model_handle = model.clone();
    let assistant = Assistant::new(AgentBuilder::new(model).tool(SystemStatusTool).build());

    let error = assistant
        .respond("Hello", &[])
        .await
        .expect_err("provider failure must be returned");

    assert!(matches!(&error.error, AssistantError::Stream(_)));
    assert!(matches!(&error.report.outcome, RunOutcome::PromptFailed));
    assert!(error.report.observations_available);
    assert_eq!(error.report.model_stages.len(), 1);
    assert_incomplete_model(&error.report.model_stages[0], 1);

    let mut source: &(dyn std::error::Error + 'static) = error.as_ref();
    let completion_error = loop {
        if let Some(completion_error) = source.downcast_ref::<CompletionError>() {
            break completion_error;
        }
        source = source
            .source()
            .expect("completion error source chain continues");
    };
    assert!(matches!(
        completion_error,
        CompletionError::ProviderError(message) if message == "provider failed"
    ));
    assert_eq!(model_handle.request_count(), 1);
}

#[test]
fn empty_final_response_is_classified_as_invalid_response() {
    let result = super::finalize_response(
        PromptResponse::new(" \n\t", Usage::new()),
        RunObserver::default(),
        Duration::ZERO,
    );

    let error = result.expect_err("blank final response must be rejected");
    assert!(matches!(&error.error, AssistantError::InvalidResponse));
    assert!(matches!(&error.report.outcome, RunOutcome::InvalidResponse));
}

#[tokio::test]
async fn visible_text_arrives_before_final_and_is_split_at_utf8_boundaries() {
    let expected_text = format!("🙂{}", "a".repeat(MAX_TEXT_DELTA_BYTES * 2));
    let stream_prefix = futures_util::stream::iter([
        Ok::<_, StreamingError>(MultiTurnStreamItem::StreamAssistantItem(
            StreamedAssistantContent::ReasoningDelta {
                id: "reasoning-1".to_string(),
                provider_id: None,
                reasoning: "private reasoning".to_string(),
            },
        )),
        Ok(MultiTurnStreamItem::StreamAssistantItem(
            StreamedAssistantContent::Text(Text::new(expected_text.clone())),
        )),
    ]);
    let (release_final, final_gate) = oneshot::channel();
    let stream_final = futures_util::stream::once(async move {
        let _ = final_gate.await;
        Ok(MultiTurnStreamItem::FinalResponse(PromptResponse::new(
            "canonical final response",
            Usage::new(),
        )))
    });
    let stream = stream_prefix.chain(stream_final);
    let (delta_tx, mut delta_rx) = mpsc::channel(4);
    let consume = tokio::spawn(consume_stream(Box::pin(stream), delta_tx));

    let mut chunks = vec![
        delta_rx
            .recv()
            .await
            .expect("the first visible delta should arrive before the final response"),
    ];
    while let Ok(delta) = delta_rx.try_recv() {
        chunks.push(delta);
    }
    assert!(
        !consume.is_finished(),
        "consumer is still waiting for the gated native final response"
    );
    assert!(
        chunks
            .iter()
            .all(|delta| delta.text.len() <= MAX_TEXT_DELTA_BYTES)
    );
    assert_eq!(
        chunks
            .iter()
            .map(|delta| delta.text.as_str())
            .collect::<String>(),
        expected_text
    );

    let _ = release_final.send(());
    let response = consume
        .await
        .expect("consumer task should settle")
        .expect("gated stream should finish");
    assert_eq!(response.output, "canonical final response");
}

#[tokio::test]
async fn stream_protocol_errors_are_typed_and_do_not_claim_success() {
    let missing_final_stream = futures_util::stream::iter([Ok::<_, StreamingError>(
        MultiTurnStreamItem::StreamAssistantItem(StreamedAssistantContent::Text(Text::new(
            "partial output",
        ))),
    )]);
    let (delta_tx, mut delta_rx) = mpsc::channel(2);
    let error = consume_stream(Box::pin(missing_final_stream), delta_tx)
        .await
        .expect_err("stream without FinalResponse must fail");
    assert!(matches!(error, AssistantError::MissingFinalResponse));
    assert_eq!(
        delta_rx
            .recv()
            .await
            .expect("partial text was streamed")
            .text,
        "partial output"
    );

    let closed_stream = futures_util::stream::iter([Ok::<_, StreamingError>(
        MultiTurnStreamItem::StreamAssistantItem(StreamedAssistantContent::Text(Text::new(
            "not deliverable",
        ))),
    )]);
    let (delta_tx, delta_rx) = mpsc::channel(1);
    drop(delta_rx);
    let error = consume_stream(Box::pin(closed_stream), delta_tx)
        .await
        .expect_err("closed delta receiver must stop stream consumption");
    assert!(matches!(error, AssistantError::DeltaReceiverClosed));
}

#[tokio::test]
async fn assistant_stream_keeps_documents_out_of_history_and_preserves_tool_report() {
    let first_usage = Usage {
        input_tokens: 11,
        output_tokens: 4,
        total_tokens: 15,
        cached_input_tokens: 0,
        cache_creation_input_tokens: 0,
        tool_use_prompt_tokens: 0,
        reasoning_tokens: 0,
    };
    let document = Document {
        id: "memory-1".to_string(),
        text: "retrieved context should not become conversation history".to_string(),
        additional_props: HashMap::new(),
    };
    let model = MockCompletionModel::from_stream_turns(vec![
        vec![
            MockStreamEvent::tool_call("tool-1", SystemStatusTool::NAME, serde_json::json!({}))
                .with_call_id("provider-call-1"),
            MockStreamEvent::final_response(first_usage),
        ],
        vec![
            MockStreamEvent::reasoning_delta("private reasoning"),
            MockStreamEvent::text("The system is online."),
            MockStreamEvent::final_response(Usage::new()),
        ],
    ]);
    let model_handle = model.clone();
    let assistant = Assistant::new(AgentBuilder::new(model).tool(SystemStatusTool).build());
    let prior_history = vec![Message::user("prior user message")];
    let (delta_tx, mut delta_rx) = mpsc::channel(8);

    let run = assistant
        .respond_stream(
            "Check the system status.",
            &prior_history,
            std::slice::from_ref(&document),
            delta_tx,
        )
        .await
        .expect("native stream with a valid tool roundtrip should succeed");
    let mut streamed_text = String::new();
    while let Some(delta) = delta_rx.recv().await {
        streamed_text.push_str(&delta.text);
    }

    assert_eq!(run.response.output, "The system is online.");
    assert_eq!(streamed_text, "The system is online.");
    assert_eq!(run.report.model_stages.len(), 2);
    assert!(run.report.model_stages[0].completed);
    assert_eq!(run.report.model_stages[0].turn, 1);
    assert_eq!(
        run.report.model_stages[0].usage,
        CallUsage::Normalized(first_usage)
    );
    assert!(run.report.model_stages[1].completed);
    assert_eq!(run.report.model_stages[1].turn, 2);
    assert_eq!(run.report.model_stages[1].usage, CallUsage::Unavailable);
    assert_eq!(run.report.tool_stages.len(), 1);
    assert_eq!(run.report.tool_stages[0].turn, 1);
    assert!(!run.report.tool_stages[0].internal_call_id.is_empty());
    assert_eq!(run.report.tool_stages[0].outcome, ToolStageOutcome::Success);

    let requests = model_handle.requests();
    assert_eq!(requests.len(), 2);
    // Recall travels in the message history, never in Rig's `documents`
    // field, so it cannot move in front of the earlier conversation.
    assert!(requests[0].documents.is_empty());
    assert!(requests[1].documents.is_empty());
    assert_eq!(requests[0].max_tokens, Some(1024));
    assert_eq!(requests[0].tools.len(), 1);
    assert!(requests[1].tools.is_empty());
    let memory_message = Message::User {
        content: vec![UserContent::document(
            document.to_string(),
            Some(DocumentMediaType::TXT),
        )],
    };
    let prompt_message = Message::user("Check the system status.");
    // Turn 1: history, memory, prompt. Turn 2 keeps that order and appends
    // the native tool call and its correlated result.
    let turn_one: Vec<Message> = requests[0].chat_history.to_vec();
    assert_eq!(
        turn_one,
        vec![
            prior_history[0].clone(),
            memory_message.clone(),
            prompt_message.clone()
        ]
    );
    let turn_two: Vec<Message> = requests[1].chat_history.to_vec();
    assert_eq!(turn_two.len(), 5);
    assert_eq!(turn_two[..3], turn_one[..]);
    assert!(matches!(
        &turn_two[3],
        Message::Assistant { content, .. }
            if matches!(content.first(), Some(AssistantContent::ToolCall(_)))
    ));
    assert!(matches!(
        &turn_two[4],
        Message::User { content }
            if matches!(content.first(), Some(UserContent::ToolResult(_)))
    ));

    let messages = run
        .response
        .messages()
        .expect("native streamed response includes the Rig transcript");
    assert!(!format!("{messages:?}").contains(&document.text));
    // Rig returns the prompt and everything after it, never the input history
    // the recall message rode in on.
    assert_eq!(messages.first(), Some(&prompt_message));
    assert!(!messages.contains(&memory_message));
    let assistant_tool_call = messages.iter().find_map(|message| {
        let Message::Assistant { content, .. } = message else {
            return None;
        };
        content.iter().find_map(|content| match content {
            AssistantContent::ToolCall(tool_call) => Some(tool_call),
            _ => None,
        })
    });
    let tool_result = messages.iter().find_map(|message| {
        let Message::User { content } = message else {
            return None;
        };
        content.iter().find_map(|content| match content {
            UserContent::ToolResult(result) => Some(result),
            _ => None,
        })
    });
    assert_eq!(
        assistant_tool_call
            .expect("Rig transcript retains native tool call")
            .id,
        tool_result
            .expect("Rig transcript retains correlated tool result")
            .call
    );
}

#[tokio::test]
async fn streaming_policy_rejection_still_records_the_completed_model_call() {
    let model = MockCompletionModel::from_stream_turns([[
        MockStreamEvent::tool_call("tool-1", SystemStatusTool::NAME, serde_json::json!({})),
        MockStreamEvent::tool_call("tool-2", SystemStatusTool::NAME, serde_json::json!({})),
        MockStreamEvent::final_response(Usage::new()),
    ]]);
    let model_handle = model.clone();
    let assistant = Assistant::new(AgentBuilder::new(model).tool(SystemStatusTool).build());
    let (delta_tx, _delta_rx) = mpsc::channel(4);

    let error = assistant
        .respond_stream("Call status twice", &[], &[], delta_tx)
        .await
        .expect_err("policy must reject multiple tool calls");

    assert!(matches!(
        &error.error,
        AssistantError::Stream(StreamingError::Prompt(_))
    ));
    assert!(matches!(&error.report.outcome, RunOutcome::PromptFailed));
    assert_eq!(model_handle.request_count(), 1);
    assert_eq!(error.report.model_stages.len(), 1);
    assert_eq!(error.report.model_stages[0].turn, 1);
    assert!(error.report.model_stages[0].completed);
    assert_eq!(error.report.model_stages[0].usage, CallUsage::Unavailable);
    assert!(error.report.tool_stages.is_empty());
}

#[tokio::test]
async fn midstream_provider_failure_returns_stream_error_and_partial_text_only() {
    let model = MockCompletionModel::from_stream_turns([[
        MockStreamEvent::text("partial output"),
        MockStreamEvent::error("stream broke"),
    ]]);
    let assistant = Assistant::new(AgentBuilder::new(model).tool(SystemStatusTool).build());
    let (delta_tx, mut delta_rx) = mpsc::channel(4);

    let error = assistant
        .respond_stream("Hello", &[], &[], delta_tx)
        .await
        .expect_err("midstream provider failure must not produce success");

    assert!(matches!(&error.error, AssistantError::Stream(_)));
    assert!(matches!(&error.report.outcome, RunOutcome::PromptFailed));
    assert_eq!(
        delta_rx
            .recv()
            .await
            .expect("visible partial text was sent")
            .text,
        "partial output"
    );
    assert_eq!(error.report.model_stages.len(), 1);
    assert_incomplete_model(&error.report.model_stages[0], 1);
}

#[tokio::test]
async fn text_deltas_split_at_the_byte_cap_and_never_inside_a_character() {
    async fn chunk_lengths(text: &str) -> Vec<usize> {
        let (deltas, mut receiver) = mpsc::channel(8);
        send_text_deltas(&deltas, text)
            .await
            .expect("receiver is open");
        drop(deltas);
        let mut lengths = Vec::new();
        while let Some(delta) = receiver.recv().await {
            lengths.push(delta.text.len());
        }
        lengths
    }
    let cap = MAX_TEXT_DELTA_BYTES;

    assert_eq!(chunk_lengths("").await, Vec::<usize>::new());
    assert_eq!(chunk_lengths(&"a".repeat(cap)).await, vec![cap]);
    assert_eq!(chunk_lengths(&"a".repeat(cap + 1)).await, vec![cap, 1]);
    // A 4-byte character that would straddle the cap moves whole to the next chunk.
    let straddling = format!("{}🙂", "a".repeat(cap - 1));
    assert_eq!(chunk_lengths(&straddling).await, vec![cap - 1, 4]);
}

/// An OpenAI-compatible streaming answer: the text, then a stop chunk.
fn text_response(text: &str) -> ResponseTemplate {
    let chunks = [
        json!({"id": "c", "object": "chat.completion.chunk", "created": 0, "model": "m",
            "choices": [{"index": 0, "delta": {"role": "assistant", "content": text},
                "finish_reason": null}]}),
        json!({"id": "c", "object": "chat.completion.chunk", "created": 0, "model": "m",
            "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]}),
    ];
    let mut body = String::new();
    for chunk in chunks {
        body.push_str(&format!("data: {chunk}\n\n"));
    }
    body.push_str("data: [DONE]\n\n");
    ResponseTemplate::new(200).set_body_raw(body, "text/event-stream")
}

/// A production assistant (system prompt included) talking to `server`.
fn wire_assistant(server: &MockServer) -> Assistant {
    let config = LocalLlmConfig {
        base_url: format!("{}/v1", server.uri()),
        health_url: format!("{}/health", server.uri()),
        model: "test-model".to_string(),
    };
    let local_llm = LocalLlm::new(&config).expect("LocalLlm should build");
    Assistant::new(
        AgentBuilder::new(local_llm.model())
            .preamble("system prompt")
            .tool(SystemStatusTool)
            .build(),
    )
}

/// Answers every chat completion with the same text.
async fn answering_server(text: &str) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(text_response(text))
        .mount(&server)
        .await;
    server
}

/// One request's messages as `(role, text)`, in wire order.
type WireMessages = Vec<(String, String)>;

/// Every recorded request's messages.
async fn wire_messages(server: &MockServer) -> Vec<WireMessages> {
    let requests = server
        .received_requests()
        .await
        .expect("request recording should be enabled");
    requests
        .iter()
        .map(|request| {
            let body: Value = request
                .body_json()
                .expect("completion request should contain JSON");
            body["messages"]
                .as_array()
                .expect("request carries a message array")
                .iter()
                .map(|message| {
                    let role = message["role"].as_str().unwrap_or_default().to_owned();
                    let text = match &message["content"] {
                        Value::String(text) => text.clone(),
                        parts => parts
                            .as_array()
                            .map(|parts| {
                                parts
                                    .iter()
                                    .filter_map(|part| part["text"].as_str())
                                    .collect::<String>()
                            })
                            .unwrap_or_default(),
                    };
                    (role, text)
                })
                .collect()
        })
        .collect()
}

fn recalled(id: &str, text: &str) -> Document {
    Document {
        id: id.to_string(),
        text: text.to_string(),
        additional_props: HashMap::new(),
    }
}

fn wire(role: &str, text: &str) -> (String, String) {
    (role.to_owned(), text.to_owned())
}

#[tokio::test]
async fn recalled_memories_sit_between_the_history_and_the_prompt_on_the_wire() {
    let history = vec![
        Message::user("earlier question"),
        Message::assistant("earlier answer"),
    ];
    let cat = recalled("m1", "My cat is Miso.");
    let tea = recalled("m2", "I drink oolong.");
    let cases: [(&[Document], WireMessages); 3] = [
        (
            &[],
            vec![
                wire("system", "system prompt"),
                wire("user", "earlier question"),
                wire("assistant", "earlier answer"),
                wire("user", "new question"),
            ],
        ),
        (
            std::slice::from_ref(&cat),
            vec![
                wire("system", "system prompt"),
                wire("user", "earlier question"),
                wire("assistant", "earlier answer"),
                wire("user", "<file id: m1>\nMy cat is Miso.\n</file>\n"),
                wire("user", "new question"),
            ],
        ),
        (
            &[cat.clone(), tea.clone()],
            vec![
                wire("system", "system prompt"),
                wire("user", "earlier question"),
                wire("assistant", "earlier answer"),
                wire(
                    "user",
                    "<file id: m1>\nMy cat is Miso.\n</file>\n<file id: m2>\nI drink oolong.\n</file>\n",
                ),
                wire("user", "new question"),
            ],
        ),
    ];

    for (documents, expected) in cases {
        let server = answering_server("ok").await;
        let assistant = wire_assistant(&server);
        assistant
            .respond_with_recall("new question", &history, documents)
            .await
            .expect("the run should succeed");

        let requests = wire_messages(&server).await;
        assert_eq!(requests, vec![expected]);
    }
}

#[tokio::test]
async fn a_changed_recall_leaves_the_earlier_conversation_as_an_exact_prefix() {
    let server = answering_server("first answer").await;
    let assistant = wire_assistant(&server);
    let first = assistant
        .respond_with_recall(
            "first question",
            &[],
            &[recalled("m1", "Recalled for the first question.")],
        )
        .await
        .expect("first run should succeed");
    // What the session would store: the run's own batch, with no recall in it.
    let history = first
        .response
        .messages()
        .expect("native streamed response includes the Rig transcript")
        .to_vec();
    assistant
        .respond_with_recall(
            "second question",
            &history,
            &[recalled("m2", "Recalled for the second question.")],
        )
        .await
        .expect("second run should succeed");

    let requests = wire_messages(&server).await;
    assert_eq!(requests.len(), 2);
    let second = &requests[1];
    assert_eq!(
        second[..3],
        [
            wire("system", "system prompt"),
            wire("user", "first question"),
            wire("assistant", "first answer"),
        ]
    );
    assert_eq!(
        second[3..],
        [
            wire(
                "user",
                "<file id: m2>\nRecalled for the second question.\n</file>\n"
            ),
            wire("user", "second question"),
        ]
    );
    assert!(
        !second
            .iter()
            .any(|(_, text)| text.contains("Recalled for the first question."))
    );
}
