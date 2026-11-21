use rig_agent::prelude::{CompletionError, CompletionModel};
use rig_core::{
    completion::{CompletionRequest, CompletionResponse, Usage},
    message::Message,
    streaming::StreamingCompletionResponse,
    test_utils::{MockCompletionModel, MockStreamEvent},
};
use serde_json::{Value, json};
use std::time::Duration;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

use crate::features::assistant::AssistantError;
use crate::{clients::local_llm::LocalLlm, config::LocalLlmConfig};

use super::{
    FactExtractor, MAX_NOTE_FACTS_OUTPUT_BYTES, NOTE_FACTS_DEADLINE, NOTE_FACTS_MAX_TOKENS,
    NOTE_FACTS_PROMPT, SAID_MARKER, render_prompt,
};

fn answer(text: &str) -> Vec<MockStreamEvent> {
    vec![
        MockStreamEvent::text(text),
        MockStreamEvent::final_response(Usage::new()),
    ]
}

#[test]
fn the_prompt_file_ends_with_exactly_one_place_for_the_users_words() {
    assert_eq!(NOTE_FACTS_PROMPT.matches(SAID_MARKER).count(), 1);
    assert!(NOTE_FACTS_PROMPT.trim_end().ends_with(SAID_MARKER));
    // The two words the parser reads must be the ones the prompt asks for.
    assert!(NOTE_FACTS_PROMPT.contains("\"FACT: \""));
    assert!(NOTE_FACTS_PROMPT.contains("\"NONE\""));
}

#[test]
fn only_the_end_of_the_prompt_changes_from_turn_to_turn() {
    let one = render_prompt("My cat is called Miso.");
    let two = render_prompt("I live in Leeds.");
    let shared = NOTE_FACTS_PROMPT
        .split_once(SAID_MARKER)
        .expect("the prompt has its marker")
        .0;
    assert!(one.starts_with(shared) && two.starts_with(shared));
    assert!(one.ends_with("My cat is called Miso.\n"));
    assert!(!one.contains(SAID_MARKER));
}

#[tokio::test]
async fn the_request_is_the_prompt_and_the_words_with_no_tools_and_no_conversation() {
    let model = MockCompletionModel::from_stream_turns([answer("NONE")]);
    let handle = model.clone();

    let text = FactExtractor::new(model)
        .extract("My cat is called Miso.")
        .await
        .expect("the run succeeds");

    assert_eq!(text, "NONE");
    let requests = handle.requests();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert!(request.tools.is_empty(), "the model is offered no tool");
    assert_eq!(request.preamble, None);
    assert_eq!(request.temperature, Some(0.0));
    assert_eq!(request.max_tokens, Some(NOTE_FACTS_MAX_TOKENS));
    assert!(request.documents.is_empty());
    assert_eq!(
        request.chat_history.iter().collect::<Vec<_>>(),
        [&Message::user(render_prompt("My cat is called Miso."))]
    );
}

#[tokio::test]
async fn the_answer_is_read_across_chunks() {
    let model = MockCompletionModel::from_stream_turns([vec![
        MockStreamEvent::text("FACT: The user's cat is "),
        MockStreamEvent::text("called Miso.\n"),
        MockStreamEvent::final_response(Usage::new()),
    ]]);
    assert_eq!(
        FactExtractor::new(model)
            .extract("x")
            .await
            .expect("the run succeeds"),
        "FACT: The user's cat is called Miso.\n"
    );
}

#[tokio::test]
async fn an_answer_is_kept_up_to_its_byte_cap_and_discarded_one_byte_over() {
    let cases = [
        ("the cap", MAX_NOTE_FACTS_OUTPUT_BYTES, true),
        ("one byte over", MAX_NOTE_FACTS_OUTPUT_BYTES + 1, false),
    ];
    for (name, bytes, kept) in cases {
        let text = "a".repeat(bytes);
        let model = MockCompletionModel::from_stream_turns([answer(&text)]);
        let read = FactExtractor::new(model)
            .extract("x")
            .await
            .expect("the run succeeds");
        assert_eq!(read, if kept { text } else { String::new() }, "{name}");
    }
}

#[tokio::test]
async fn a_failed_stream_and_a_stream_with_no_end_are_errors() {
    let failing = MockCompletionModel::from_stream_turns([vec![
        MockStreamEvent::text("FACT: partial"),
        MockStreamEvent::error("injected failure"),
    ]]);
    assert!(matches!(
        FactExtractor::new(failing).extract("x").await,
        Err(AssistantError::Stream(_))
    ));

    let unfinished =
        MockCompletionModel::from_stream_turns([vec![MockStreamEvent::text("FACT: partial")]]);
    // Rig reports the missing end itself; either way no text comes back as an answer.
    assert!(matches!(
        FactExtractor::new(unfinished).extract("x").await,
        Err(AssistantError::Stream(_) | AssistantError::MissingFinalResponse)
    ));
}

#[derive(Clone)]
struct PendingModel;

impl CompletionModel for PendingModel {
    async fn completion(
        &self,
        _request: CompletionRequest,
    ) -> Result<CompletionResponse, CompletionError> {
        unreachable!("the extraction streams")
    }

    async fn stream(
        &self,
        _request: CompletionRequest,
    ) -> Result<StreamingCompletionResponse, CompletionError> {
        std::future::pending().await
    }
}

#[tokio::test(start_paused = true)]
async fn a_model_that_never_answers_times_out_at_the_deadline() {
    let extractor = FactExtractor::new(PendingModel);
    let mut run = std::pin::pin!(extractor.extract("x"));

    // One millisecond short of the deadline it is still waiting.
    let early = tokio::time::timeout(NOTE_FACTS_DEADLINE - Duration::from_millis(1), &mut run);
    assert!(early.await.is_err(), "still waiting before the deadline");
    assert!(matches!(run.await, Err(AssistantError::RunTimeout)));
}

fn sse(chunks: &[Value]) -> ResponseTemplate {
    let mut body = String::new();
    for chunk in chunks {
        body.push_str(&format!("data: {chunk}\n\n"));
    }
    body.push_str("data: [DONE]\n\n");
    ResponseTemplate::new(200).set_body_raw(body, "text/event-stream")
}

fn chunk(delta: Value, finish_reason: Option<&str>) -> Value {
    json!({"id": "c", "object": "chat.completion.chunk", "created": 0, "model": "m",
        "choices": [{"index": 0, "delta": delta, "finish_reason": finish_reason}]})
}

fn extractor_for(server: &MockServer) -> FactExtractor {
    let config = LocalLlmConfig {
        base_url: format!("{}/v1", server.uri()),
        health_url: format!("{}/health", server.uri()),
        model: "test-model".to_owned(),
    };
    FactExtractor::new(LocalLlm::new(&config).expect("LocalLlm builds").model())
}

#[tokio::test]
async fn an_answer_the_server_cut_short_is_discarded_whole() {
    let cases = [
        ("finished", "stop", true),
        ("token limit", "length", false),
        ("filtered", "content_filter", false),
    ];
    let text = "FACT: The user likes tea.";
    for (name, reason, kept) in cases {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(sse(&[
                chunk(json!({"role": "assistant", "content": text}), None),
                chunk(json!({}), Some(reason)),
            ]))
            .mount(&server)
            .await;

        let read = extractor_for(&server)
            .extract("I like tea")
            .await
            .expect("the run completes");

        assert_eq!(read, if kept { text } else { "" }, "{name}");
    }
}

#[tokio::test]
async fn on_the_wire_the_request_has_one_user_message_no_tools_and_streams() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(sse(&[
            chunk(json!({"role": "assistant", "content": "NONE"}), None),
            chunk(json!({}), Some("stop")),
        ]))
        .mount(&server)
        .await;

    extractor_for(&server)
        .extract("I like tea")
        .await
        .expect("the run completes");

    let requests = server.received_requests().await.expect("recording is on");
    assert_eq!(requests.len(), 1);
    let body: Value = requests[0].body_json().expect("a JSON body");
    let messages = body["messages"].as_array().expect("messages");
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["role"], "user");
    assert!(
        messages[0]["content"]
            .to_string()
            .contains("The user said:\\nI like tea")
    );
    assert!(
        body.get("tools")
            .is_none_or(|tools| tools.as_array().is_some_and(Vec::is_empty))
    );
    assert_eq!(body["stream"], true);
    assert_eq!(body["temperature"], 0.0);
    assert_eq!(body["max_tokens"], NOTE_FACTS_MAX_TOKENS);
}
