use std::net::SocketAddr;

use rig_agent::AgentBuilder;
use rig_agent::completion::PromptError;
use rig_core::{
    completion::{CompletionModel, CompletionRequest},
    message::{AssistantContent, Message, UserContent},
};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

use crate::{
    assistant::{Assistant, CallUsage},
    clients::local_llm::{
        LocalEndpointError, LocalLlm, validate_local_endpoint, validate_resolved_addresses,
    },
    config::{AppConfig, OptionalTtsConfig},
    tools::system_status_tool::SystemStatusTool,
};

#[test]
fn local_endpoint_guard_allows_local_ip_targets_and_rejects_public_destinations() {
    for endpoint in [
        "http://localhost:1234/v1",
        "http://127.0.0.1:1234/v1",
        "http://192.168.1.40:1234/v1",
        "https://[::1]:1234/v1",
        "https://[fd12:3456::2]:1234/v1",
        "http://[fe80::1]:1234/v1",
        "http://jarvis-ai.local:8080/v1",
        "http://upper.JARVIS.local.:8080/v1",
        "http://assistant.lab.local:8080/v1",
    ] {
        assert!(
            validate_local_endpoint(endpoint).is_ok(),
            "explicit local endpoint should be accepted"
        );
    }

    assert_eq!(
        validate_local_endpoint("https://api.example.com/v1"),
        Err(LocalEndpointError::NonLocalHost)
    );
    for host in [
        "jarvis.local.example",
        ".local",
        "jarvis..local",
        "-jarvis.local",
        "jarvis-.local",
        "jarvis_ai.local",
    ] {
        assert!(
            validate_local_endpoint(&format!("http://{host}:8080/v1")).is_err(),
            "invalid .local hostname should be rejected: {host}"
        );
    }
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
fn local_dns_results_must_be_nonempty_and_exclusively_local() {
    let loopback: SocketAddr = "127.0.0.1:0".parse().expect("valid loopback socket");
    let private: SocketAddr = "192.168.1.20:0".parse().expect("valid private socket");
    let public: SocketAddr = "8.8.8.8:0".parse().expect("valid public socket");

    assert_eq!(
        validate_resolved_addresses(vec![loopback, private]),
        Some(vec![loopback, private])
    );
    assert_eq!(validate_resolved_addresses(Vec::new()), None);
    assert_eq!(validate_resolved_addresses(vec![public]), None);
    assert_eq!(validate_resolved_addresses(vec![loopback, public]), None);
}

async fn request_bodies(server: &MockServer) -> Vec<rig_core::serde_json::Value> {
    let requests = server
        .received_requests()
        .await
        .expect("request recording should be enabled");

    requests
        .iter()
        .map(|request| {
            assert_eq!(request.method.as_str(), "POST");
            assert!(
                matches!(
                    request.url.path(),
                    "/chat/completions" | "/v1/chat/completions"
                ),
                "unexpected completion path: {}",
                request.url.path()
            );
            request
                .body_json()
                .expect("completion request should contain JSON")
        })
        .collect()
}

fn prompt_error<'a>(error: &'a (dyn std::error::Error + 'static)) -> &'a PromptError {
    let mut source = error;
    loop {
        if let Some(prompt_error) = source.downcast_ref::<PromptError>() {
            return prompt_error;
        }
        source = source
            .source()
            .expect("assistant error chain should retain the Rig prompt error");
    }
}

#[tokio::test]
async fn openai_adapter_parses_plain_text_response() {
    let server = MockServer::start().await;
    let port = reqwest::Url::parse(&server.uri())
        .expect("WireMock URI should be valid")
        .port()
        .expect("WireMock should use a random port");

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(rig_core::serde_json::json!({
                "id": "chatcmpl-test",
                "object": "chat.completion",
                "created": 0,
                "model": "test-model",
                "choices": [
                    {
                        "index": 0,
                        "message": {
                            "role": "assistant",
                            "content": "JARVIS ONLINE"
                        },
                        "finish_reason": "stop"
                    }
                ],
                "usage": {
                    "prompt_tokens": 1,
                    "completion_tokens": 1,
                    "total_tokens": 2
                }
            })),
        )
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let config = AppConfig {
        local_llm_base_url: format!("http://localhost:{port}/v1"),
        local_llm_health_url: format!("http://localhost:{port}/health"),
        local_llm_model: "test-model".to_string(),
        archive_path: "data/jarvis.sqlite3".to_string(),
        embedding_model_dir: "data/embeddings".into(),
        memory_index_dir: "data/memory-index".into(),
        tts: OptionalTtsConfig::Disabled,
    };

    let local_llm = LocalLlm::new(&config).expect("LocalLlm should build");
    let model = local_llm.model();

    let request = CompletionRequest {
        preamble: None,
        chat_history: vec![Message::user("Say JARVIS ONLINE")],
        documents: vec![],
        tools: vec![],
        temperature: None,
        max_tokens: None,
        tool_choice: None,
        additional_params: None,

        model: None,
        output_schema: None,
        record_telemetry_content: false,
    };
    let response = model
        .completion(request)
        .await
        .expect("OpenAI response should deserialize");

    let text = response
        .choice
        .iter()
        .find_map(|content| match content {
            AssistantContent::Text(text) => Some(text.text.as_str()),
            _ => None,
        })
        .expect("Rig should parse assistant text");

    assert_eq!(text, "JARVIS ONLINE");
    assert_eq!(request_bodies(&server).await.len(), 1);

    let requests = server
        .received_requests()
        .await
        .expect("WireMock should record completion request");
    let completion = requests
        .iter()
        .find(|request| request.method.as_str() == "POST")
        .expect("localhost should reach the completion fixture");
    assert_eq!(completion.url.path(), "/v1/chat/completions");
    let expected_host = format!("localhost:{port}");
    assert_eq!(
        completion
            .headers
            .get("host")
            .and_then(|value| value.to_str().ok()),
        Some(expected_host.as_str())
    );

    local_llm
        .health_check()
        .await
        .expect("localhost health check should reach the same fixture");
    let requests = server
        .received_requests()
        .await
        .expect("WireMock should record health request");
    let health = requests
        .iter()
        .find(|request| request.method.as_str() == "GET" && request.url.path() == "/health")
        .expect("health client should preserve its configured path");
    assert_eq!(
        health
            .headers
            .get("host")
            .and_then(|value| value.to_str().ok()),
        Some(expected_host.as_str())
    );
}

#[tokio::test]
async fn openai_adapter_parses_system_status_tool_call() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(rig_core::serde_json::json!({
                "id": "chatcmpl-tool-test",
                "object": "chat.completion",
                "created": 0,
                "model": "test-model",
                "choices": [
                    {
                        "index": 0,
                        "message": {
                            "role": "assistant",
                            "content": null,
                            "tool_calls": [
                                {
                                    "id": "call-1",
                                    "type": "function",
                                    "function": {
                                        "name": "system_status",
                                        "arguments": "{}"
                                    }
                                }
                            ]
                        },
                        "finish_reason": "tool_calls"
                    }
                ],
                "usage": {
                    "prompt_tokens": 1,
                    "completion_tokens": 1,
                    "total_tokens": 2
                }
            })),
        )
        .mount(&server)
        .await;

    let config = AppConfig {
        local_llm_base_url: server.uri(),
        local_llm_health_url: format!("{}/health", server.uri()),
        local_llm_model: "test-model".to_string(),
        archive_path: "data/jarvis.sqlite3".to_string(),
        embedding_model_dir: "data/embeddings".into(),
        memory_index_dir: "data/memory-index".into(),
        tts: OptionalTtsConfig::Disabled,
    };

    let local_llm = LocalLlm::new(&config).expect("LocalLlm should build");

    let model = local_llm.model();

    let request = CompletionRequest {
        preamble: None,
        chat_history: vec![Message::user("Check the system status")],
        documents: vec![],
        tools: vec![],
        temperature: None,
        max_tokens: None,
        tool_choice: None,
        additional_params: None,
        model: None,
        output_schema: None,
        record_telemetry_content: false,
    };

    let response = model
        .completion(request)
        .await
        .expect("OpenAI tool-call response should deserialize");

    let tool_call = response
        .choice
        .iter()
        .find_map(|content| {
            if let AssistantContent::ToolCall(tool_call) = content {
                Some(tool_call)
            } else {
                None
            }
        })
        .expect("expected Rig to parse a tool call");

    assert_eq!(tool_call.function.name, "system_status");
    assert_eq!(tool_call.id, "call-1");
    assert_eq!(
        tool_call.function.arguments,
        rig_core::serde_json::json!({})
    );
    assert_eq!(request_bodies(&server).await.len(), 1);
}

#[tokio::test]
async fn openai_adapter_completes_system_status_roundtrip() {
    let server = MockServer::start().await;

    let tool_call_response =
        ResponseTemplate::new(200).set_body_json(rig_core::serde_json::json!({
            "id": "chatcmpl-tool",
            "object": "chat.completion",
            "created": 0,
            "model": "test-model",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call-1",
                        "type": "function",
                        "function": {
                            "name": "system_status",
                            "arguments": "{}"
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": {
                "prompt_tokens": 1,
                "completion_tokens": 1,
                "total_tokens": 2
            }
        }));

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(tool_call_response)
        .up_to_n_times(1)
        .mount(&server)
        .await;

    let final_response = ResponseTemplate::new(200).set_body_json(rig_core::serde_json::json!({
        "id": "chatcmpl-final",
        "object": "chat.completion",
        "created": 0,
        "model": "test-model",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": "System status retrieved."
            },
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 1,
            "completion_tokens": 1,
            "total_tokens": 2
        }
    }));

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(final_response)
        .mount(&server)
        .await;
    let config = AppConfig {
        local_llm_base_url: server.uri(),
        local_llm_health_url: format!("{}/health", server.uri()),
        local_llm_model: "test-model".to_string(),
        archive_path: "data/jarvis.sqlite3".to_string(),
        embedding_model_dir: "data/embeddings".into(),
        memory_index_dir: "data/memory-index".into(),
        tts: OptionalTtsConfig::Disabled,
    };

    let local_llm = LocalLlm::new(&config).expect("LocalLlm should build");

    let agent = AgentBuilder::new(local_llm.model())
        .tool(SystemStatusTool)
        .build();

    let assistant = Assistant::new(agent);

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
    assert_eq!(first_request["max_tokens"], 1026);

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
    assert_eq!(second_request["max_tokens"], 1026);

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

#[tokio::test]
async fn openai_adapter_rejects_length_terminated_tool_call() {
    let server = MockServer::start().await;

    let tool_call_response =
        ResponseTemplate::new(200).set_body_json(rig_core::serde_json::json!({
            "id": "chatcmpl-tool",
            "object": "chat.completion",
            "created": 0,
            "model": "test-model",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call-1",
                        "type": "function",
                        "function": {
                            "name": "system_status",
                            "arguments": "{}"
                        }
                    }]
                },
                "finish_reason": "length"
            }],
            "usage": {
                "prompt_tokens": 1,
                "completion_tokens": 1,
                "total_tokens": 2
            }
        }));

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(tool_call_response)
        .up_to_n_times(1)
        .mount(&server)
        .await;

    let config = AppConfig {
        local_llm_base_url: server.uri(),
        local_llm_health_url: format!("{}/health", server.uri()),
        local_llm_model: "test-model".to_string(),
        archive_path: "data/jarvis.sqlite3".to_string(),
        embedding_model_dir: "data/embeddings".into(),
        memory_index_dir: "data/memory-index".into(),
        tts: OptionalTtsConfig::Disabled,
    };

    let local_llm = LocalLlm::new(&config).expect("LocalLlm should build");

    let agent = AgentBuilder::new(local_llm.model())
        .tool(SystemStatusTool)
        .build();

    let assistant = Assistant::new(agent);

    let result = assistant.respond("Check the system status.", &[]).await;
    let error = result.expect_err("length-terminated response must be rejected");
    assert!(matches!(
        prompt_error(error.as_ref()),
        PromptError::PromptCancelled { reason, .. }
            if reason == "Phase 4 policy: FinishReason"
    ));
    assert_eq!(request_bodies(&server).await.len(), 1);
}

#[tokio::test]
async fn openai_adapter_rejects_content_filtered_tool_call() {
    let server = MockServer::start().await;

    let response = ResponseTemplate::new(200).set_body_json(rig_core::serde_json::json!({
        "id": "chatcmpl-filtered",
        "object": "chat.completion",
        "created": 0,
        "model": "test-model",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": null,
                "tool_calls": [{
                    "id": "call-1",
                    "type": "function",
                    "function": {
                        "name": "system_status",
                        "arguments": "{}"
                    }
                }]
            },
            "finish_reason": "content_filter"
        }],
        "usage": {
            "prompt_tokens": 1,
            "completion_tokens": 1,
            "total_tokens": 2
        }
    }));

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(response)
        .expect(1)
        .mount(&server)
        .await;

    let config = AppConfig {
        local_llm_base_url: server.uri(),
        local_llm_health_url: format!("{}/health", server.uri()),
        local_llm_model: "test-model".to_string(),
        archive_path: "data/jarvis.sqlite3".to_string(),
        embedding_model_dir: "data/embeddings".into(),
        memory_index_dir: "data/memory-index".into(),
        tts: OptionalTtsConfig::Disabled,
    };

    let local_llm = LocalLlm::new(&config).expect("LocalLlm should build");

    let agent = AgentBuilder::new(local_llm.model())
        .tool(SystemStatusTool)
        .build();

    let assistant = Assistant::new(agent);

    let result = assistant.respond("Check the system status.", &[]).await;
    let error = result.expect_err("content-filtered response must be rejected");
    assert!(matches!(
        prompt_error(error.as_ref()),
        PromptError::PromptCancelled { reason, .. }
            if reason == "Phase 4 policy: FinishReason"
    ));
    assert_eq!(request_bodies(&server).await.len(), 1);
}

#[tokio::test]
async fn openai_adapter_rejects_unknown_finish_reason() {
    let server = MockServer::start().await;

    let response = ResponseTemplate::new(200).set_body_json(rig_core::serde_json::json!({
        "id": "chatcmpl-other",
        "object": "chat.completion",
        "created": 0,
        "model": "test-model",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": null,
                "tool_calls": [{
                    "id": "call-1",
                    "type": "function",
                    "function": {
                        "name": "system_status",
                        "arguments": "{}"
                    }
                }]
            },
            "finish_reason": "provider_specific_reason"
        }],
        "usage": {
            "prompt_tokens": 1,
            "completion_tokens": 1,
            "total_tokens": 2
        }
    }));

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(response)
        .expect(1)
        .mount(&server)
        .await;

    let config = AppConfig {
        local_llm_base_url: server.uri(),
        local_llm_health_url: format!("{}/health", server.uri()),
        local_llm_model: "test-model".to_string(),
        archive_path: "data/jarvis.sqlite3".to_string(),
        embedding_model_dir: "data/embeddings".into(),
        memory_index_dir: "data/memory-index".into(),
        tts: OptionalTtsConfig::Disabled,
    };

    let local_llm = LocalLlm::new(&config).expect("LocalLlm should build");

    let agent = AgentBuilder::new(local_llm.model())
        .tool(SystemStatusTool)
        .build();

    let assistant = Assistant::new(agent);

    let result = assistant.respond("Check the system status.", &[]).await;
    let error = result.expect_err("unknown finish reason must be rejected");
    assert!(matches!(
        prompt_error(error.as_ref()),
        PromptError::PromptCancelled { reason, .. }
            if reason == "Phase 4 policy: FinishReason"
    ));
    assert_eq!(request_bodies(&server).await.len(), 1);
}

#[tokio::test]
async fn openai_adapter_rejects_unknown_tool_call() {
    let server = MockServer::start().await;

    let response = ResponseTemplate::new(200).set_body_json(rig_core::serde_json::json!({
        "id": "chatcmpl-unknown-tool",
        "object": "chat.completion",
        "created": 0,
        "model": "test-model",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": null,
                "tool_calls": [{
                    "id": "call-1",
                    "type": "function",
                    "function": {
                        "name": "reboot_system",
                        "arguments": "{}"
                    }
                }]
            },
            "finish_reason": "tool_calls"
        }],
        "usage": {
            "prompt_tokens": 1,
            "completion_tokens": 1,
            "total_tokens": 2
        }
    }));

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(response)
        .expect(1)
        .mount(&server)
        .await;

    let config = AppConfig {
        local_llm_base_url: server.uri(),
        local_llm_health_url: format!("{}/health", server.uri()),
        local_llm_model: "test-model".to_string(),
        archive_path: "data/jarvis.sqlite3".to_string(),
        embedding_model_dir: "data/embeddings".into(),
        memory_index_dir: "data/memory-index".into(),
        tts: OptionalTtsConfig::Disabled,
    };

    let local_llm = LocalLlm::new(&config).expect("LocalLlm should build");

    // Only system_status is registered.
    let agent = AgentBuilder::new(local_llm.model())
        .tool(SystemStatusTool)
        .build();

    let assistant = Assistant::new(agent);

    let result = assistant.respond("Reboot the system.", &[]).await;
    let error = result.expect_err("unregistered tool must be rejected");
    assert!(matches!(
        prompt_error(error.as_ref()),
        PromptError::PromptCancelled { reason, .. }
            if reason == "Invalid tool call rejected by Phase 4 policy"
    ));
    assert_eq!(request_bodies(&server).await.len(), 1);
}

#[tokio::test]
async fn openai_adapter_rejects_invalid_system_status_arguments() {
    let server = MockServer::start().await;

    let response = ResponseTemplate::new(200).set_body_json(rig_core::serde_json::json!({
        "id": "chatcmpl-invalid-args",
        "object": "chat.completion",
        "created": 0,
        "model": "test-model",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": null,
                "tool_calls": [{
                    "id": "call-1",
                    "type": "function",
                    "function": {
                        "name": "system_status",
                        "arguments": "{\"unexpected\":true}"
                    }
                }]
            },
            "finish_reason": "tool_calls"
        }],
        "usage": {
            "prompt_tokens": 1,
            "completion_tokens": 1,
            "total_tokens": 2
        }
    }));

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(response)
        .expect(1)
        .mount(&server)
        .await;

    let config = AppConfig {
        local_llm_base_url: server.uri(),
        local_llm_health_url: format!("{}/health", server.uri()),
        local_llm_model: "test-model".to_string(),
        archive_path: "data/jarvis.sqlite3".to_string(),
        embedding_model_dir: "data/embeddings".into(),
        memory_index_dir: "data/memory-index".into(),
        tts: OptionalTtsConfig::Disabled,
    };

    let local_llm = LocalLlm::new(&config).expect("LocalLlm should build");

    let agent = AgentBuilder::new(local_llm.model())
        .tool(SystemStatusTool)
        .build();

    let assistant = Assistant::new(agent);

    let result = assistant.respond("Check the system status.", &[]).await;
    let error = result.expect_err("invalid system_status arguments must be rejected");
    assert!(matches!(
        prompt_error(error.as_ref()),
        PromptError::PromptCancelled { reason, .. }
            if reason == "Phase 4 policy: ToolArguments"
    ));
    assert_eq!(request_bodies(&server).await.len(), 1);
}
