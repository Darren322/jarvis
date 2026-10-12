use std::net::SocketAddr;

use rig_agent::AgentBuilder;
use rig_core::{
    message::{Message, UserContent},
    serde_json::{Value, json},
};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

use crate::{
    clients::local_llm::{
        LocalEndpointError, LocalLlm, validate_local_endpoint, validate_resolved_addresses,
    },
    config::LocalLlmConfig,
    features::assistant::{Assistant, CallUsage},
    tools::system_status_tool::SystemStatusTool,
};

/// Each pair sits on either side of an allowed range or a hostname rule.
#[rustfmt::skip]
#[test]
fn local_endpoint_guard_accepts_exactly_the_local_ranges() {
    let label_63 = "a".repeat(63);
    let label_64 = "a".repeat(64);
    let host_253 = format!("{}.local", vec!["a".repeat(61); 4].join("."));
    let host_254 = format!("a{host_253}");
    assert_eq!((host_253.len(), host_254.len()), (253, 254));

    let cases: Vec<(String, bool)> = vec![
        // IPv4: loopback, the three private ranges, and link-local, each
        // checked at its edge.
        ("127.0.0.1".into(), true),       ("128.0.0.1".into(), false),
        ("10.255.255.255".into(), true),  ("11.0.0.0".into(), false),
        ("172.16.0.0".into(), true),      ("172.15.255.255".into(), false),
        ("172.31.255.255".into(), true),  ("172.32.0.0".into(), false),
        ("192.168.0.1".into(), true),     ("192.169.0.1".into(), false),
        ("169.254.0.1".into(), true),     ("169.255.0.1".into(), false),
        // IPv6: loopback, unique-local fc00::/7, link-local fe80::/10.
        ("[::1]".into(), true),           ("[::2]".into(), false),
        ("[fc00::1]".into(), true),       ("[fbff::1]".into(), false),
        ("[fdff::1]".into(), true),       ("[fe00::1]".into(), false),
        ("[fe80::1]".into(), true),       ("[fec0::1]".into(), false),
        // Names: localhost and well-formed .local names only.
        ("localhost".into(), true),       ("api.example.com".into(), false),
        ("jarvis-ai.local".into(), true), ("jarvis-ai.local.example".into(), false),
        ("UPPER.Local.".into(), true),    (".local".into(), false),
        (format!("{label_63}.local"), true), (format!("{label_64}.local"), false),
        (host_253, true),                 (host_254, false),
        ("a-b.local".into(), true),       ("-ab.local".into(), false),
        ("ab.local".into(), true),        ("ab-.local".into(), false),
        ("ab.local".into(), true),        ("a_b.local".into(), false),
    ];

    for (host, accepted) in cases {
        let result = validate_local_endpoint(&format!("http://{host}:8080/v1"));
        assert_eq!(result.is_ok(), accepted, "{host}: {result:?}");
    }
}

#[test]
fn endpoint_urls_reject_credentials_and_non_http_schemes() {
    assert!(validate_local_endpoint("https://127.0.0.1:1234/v1").is_ok());
    assert_eq!(
        validate_local_endpoint("http://user:secret@127.0.0.1:1234/v1"),
        Err(LocalEndpointError::CredentialsNotAllowed)
    );
    assert_eq!(
        validate_local_endpoint("file:///tmp/model"),
        Err(LocalEndpointError::UnsupportedScheme)
    );
}

#[test]
fn dns_results_must_be_nonempty_and_every_address_local() {
    let local: SocketAddr = "192.168.1.20:0".parse().expect("valid socket");
    let public: SocketAddr = "8.8.8.8:0".parse().expect("valid socket");

    assert_eq!(validate_resolved_addresses(vec![local]), Some(vec![local]));
    assert_eq!(validate_resolved_addresses(Vec::new()), None);
    // One public address among local ones poisons the whole set.
    assert_eq!(validate_resolved_addresses(vec![local, public]), None);
}

async fn request_bodies(server: &MockServer) -> Vec<Value> {
    let requests = server
        .received_requests()
        .await
        .expect("request recording should be enabled");

    requests
        .iter()
        .filter(|request| request.method.as_str() == "POST")
        .map(|request| {
            assert_eq!(request.url.path(), "/v1/chat/completions");
            request
                .body_json()
                .expect("completion request should contain JSON")
        })
        .collect()
}

/// One OpenAI `chat.completion.chunk` carrying `delta`.
fn chunk(delta: Value, finish_reason: Option<&str>) -> Value {
    json!({
        "id": "chatcmpl-test",
        "object": "chat.completion.chunk",
        "created": 0,
        "model": "test-model",
        "choices": [{"index": 0, "delta": delta, "finish_reason": finish_reason}],
    })
}

/// The trailing usage-only chunk llama.cpp sends when usage is requested.
fn usage_chunk() -> Value {
    json!({
        "id": "chatcmpl-test",
        "object": "chat.completion.chunk",
        "created": 0,
        "model": "test-model",
        "choices": [],
        "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2},
    })
}

/// A server-sent-events body, as a streaming OpenAI-compatible server returns.
fn sse(chunks: &[Value]) -> ResponseTemplate {
    let mut body = String::new();
    for chunk in chunks {
        body.push_str(&format!("data: {chunk}\n\n"));
    }
    body.push_str("data: [DONE]\n\n");
    ResponseTemplate::new(200).set_body_raw(body, "text/event-stream")
}

fn text_stream(text: &str) -> ResponseTemplate {
    sse(&[
        chunk(json!({"role": "assistant", "content": text}), None),
        chunk(json!({}), Some("stop")),
        usage_chunk(),
    ])
}

/// Builds the production client against `origin` (scheme, host and port).
fn local_assistant(origin: &str) -> (LocalLlm, Assistant) {
    let config = LocalLlmConfig {
        base_url: format!("{origin}/v1"),
        health_url: format!("{origin}/health"),
        model: "test-model".to_string(),
    };
    let local_llm = LocalLlm::new(&config).expect("LocalLlm should build");
    let assistant = Assistant::new(
        AgentBuilder::new(local_llm.model())
            .tool(SystemStatusTool)
            .build(),
    );
    (local_llm, assistant)
}

#[tokio::test]
async fn streams_text_from_a_localhost_endpoint_and_health_uses_its_own_url() {
    let server = MockServer::start().await;
    let port = reqwest::Url::parse(&server.uri())
        .expect("WireMock URI should be valid")
        .port()
        .expect("WireMock should use a random port");
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(sse(&[
            chunk(json!({"role": "assistant", "content": "JARVIS "}), None),
            chunk(json!({"content": "ONLINE"}), None),
            chunk(json!({}), Some("stop")),
            usage_chunk(),
        ]))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    // `localhost` exercises the local-only DNS resolver end to end.
    let (local_llm, assistant) = local_assistant(&format!("http://localhost:{port}"));
    let run = assistant
        .respond("Reply with exactly: JARVIS ONLINE", &[])
        .await
        .expect("a streamed text answer succeeds");
    assert_eq!(run.response.output, "JARVIS ONLINE");

    let bodies = request_bodies(&server).await;
    assert_eq!(bodies.len(), 1);
    assert_eq!(bodies[0]["stream"], true);
    assert_eq!(bodies[0]["max_tokens"], 1024);

    local_llm
        .health_check()
        .await
        .expect("health check reaches its configured path");
    let requests = server
        .received_requests()
        .await
        .expect("WireMock records requests");
    let expected_host = format!("localhost:{port}");
    for request in &requests {
        assert_eq!(
            request
                .headers
                .get("host")
                .and_then(|value| value.to_str().ok()),
            Some(expected_host.as_str())
        );
    }
    assert!(
        requests
            .iter()
            .any(|request| request.method.as_str() == "GET" && request.url.path() == "/health")
    );
}

#[tokio::test]
async fn streams_a_tool_call_roundtrip_with_a_correlated_result() {
    let server = MockServer::start().await;
    // The tool call arrives as deltas: id and name first, arguments after.
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(sse(&[
            chunk(
                json!({"role": "assistant", "tool_calls": [{
                    "index": 0, "id": "call-1", "type": "function",
                    "function": {"name": "system_status", "arguments": ""}
                }]}),
                None,
            ),
            chunk(
                json!({"tool_calls": [{"index": 0, "function": {"arguments": "{}"}}]}),
                None,
            ),
            chunk(json!({}), Some("tool_calls")),
            usage_chunk(),
        ]))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(text_stream("System status retrieved."))
        .mount(&server)
        .await;

    let (_local_llm, assistant) = local_assistant(&server.uri());

    let response = assistant
        .respond("Check the system status.", &[])
        .await
        .expect("system status roundtrip should succeed");

    assert_eq!(response.response.output, "System status retrieved.");
    assert_eq!(response.response.requests(), 2);
    assert!(response.report.observations_available);
    assert_eq!(response.report.model_stages.len(), 2);
    let expected_usage = rig_core::completion::Usage {
        input_tokens: 1,
        output_tokens: 1,
        total_tokens: 2,
        cached_input_tokens: 0,
        cache_creation_input_tokens: 0,
        tool_use_prompt_tokens: 0,
        reasoning_tokens: 0,
    };
    for stage in &response.report.model_stages {
        assert!(stage.completed);
        match &stage.usage {
            CallUsage::Normalized(usage) => assert_eq!(usage, &expected_usage),
            CallUsage::Unavailable => panic!("OpenAI usage should reach the run report"),
        }
    }

    let request_bodies = request_bodies(&server).await;
    assert_eq!(request_bodies.len(), 2);
    let first_request = &request_bodies[0];
    assert_eq!(first_request["tool_choice"], "auto");
    assert_eq!(first_request["max_tokens"], 1024);

    let first_tools = first_request["tools"]
        .as_array()
        .expect("first request should offer the system status tool");
    assert_eq!(first_tools.len(), 1);
    assert_eq!(
        first_tools[0],
        rig_core::serde_json::json!({
            "type": "function",
            "function": {
                "name": "system_status",
                "description": "Returns the current system status, including hostname, uptime, and memory usage.",
                "parameters": {
                    "type": "object",
                    "properties": {},
                    "additionalProperties": false
                }
            }
        })
    );

    let second_request = &request_bodies[1];
    assert!(second_request.get("tools").is_none());
    assert_eq!(second_request["tool_choice"], "none");
    assert_eq!(second_request["max_tokens"], 1024);

    let second_messages = second_request["messages"]
        .as_array()
        .expect("second request should include conversation history");
    let assistant_messages = second_messages
        .iter()
        .filter(|message| message["role"] == "assistant" && message["tool_calls"].is_array())
        .collect::<Vec<_>>();
    assert_eq!(assistant_messages.len(), 1);
    let assistant_tool_calls = assistant_messages[0]["tool_calls"]
        .as_array()
        .expect("assistant message should contain tool calls");
    assert_eq!(assistant_tool_calls.len(), 1);
    let assistant_tool_call = &assistant_tool_calls[0];
    assert_eq!(assistant_tool_call["id"], "call-1");
    assert_eq!(assistant_tool_call["function"]["name"], "system_status");
    assert_eq!(
        rig_core::serde_json::from_str::<rig_core::serde_json::Value>(
            assistant_tool_call["function"]["arguments"]
                .as_str()
                .expect("tool call arguments should be JSON text")
        )
        .expect("tool call arguments should parse as JSON"),
        rig_core::serde_json::json!({})
    );

    let tool_results = second_messages
        .iter()
        .filter(|message| message["role"] == "tool")
        .collect::<Vec<_>>();
    assert_eq!(tool_results.len(), 1);
    let tool_result = tool_results[0];
    assert_eq!(tool_result["tool_call_id"], "call-1");

    let status_json: rig_core::serde_json::Value = rig_core::serde_json::from_str(
        tool_result["content"]
            .as_str()
            .expect("system status tool result should serialize as JSON text"),
    )
    .expect("system status tool result should contain structured JSON");
    assert!(status_json.get("source_node").is_some());
    assert!(status_json["source_node"].is_null() || status_json["source_node"].is_string());
    assert!(status_json["observed_at_unix_ms"].as_i64().is_some());
    assert!(status_json["uptime_seconds"].as_u64().is_some());
    let total_memory_bytes = status_json["total_memory_bytes"]
        .as_u64()
        .expect("total_memory_bytes should be an unsigned integer");
    let used_memory_bytes = status_json["used_memory_bytes"]
        .as_u64()
        .expect("used_memory_bytes should be an unsigned integer");
    assert!(total_memory_bytes > 0);
    assert!(used_memory_bytes <= total_memory_bytes);

    let transcript_tool_result = response
        .response
        .messages
        .as_ref()
        .expect("assistant response should retain the run transcript")
        .iter()
        .find_map(|message| match message {
            Message::User { content } => content.iter().find_map(|content| match content {
                UserContent::ToolResult(result) if result.call.as_str() == "call-1" => Some(result),
                _ => None,
            }),
            _ => None,
        })
        .expect("transcript should retain the matching tool result");
    assert_eq!(transcript_tool_result.name, "system_status");
    let transcript_status = transcript_tool_result
        .content
        .iter()
        .find_map(|content| content.as_json())
        .expect("transcript should retain structured status JSON");
    assert_eq!(transcript_status, &status_json);
}
