use rig_agent::AgentBuilder;
use rig_core::{
    completion::{CompletionModel, CompletionRequest},
    message::{AssistantContent, Message},
};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

use crate::{
    clients::local_llm::LocalLlm, config::AppConfig, services::assistant::Assistant,
    tools::system_status_tool::SystemStatusTool,
};

#[tokio::test]
async fn openai_adapter_parses_plain_text_response() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
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

    let config = AppConfig {
        local_llm_base_url: server.uri(),
        local_llm_health_url: format!("{}/health", server.uri()),
        local_llm_model: "test-model".to_string(),
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

    assert_eq!(response.choice.len(), 1);
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
    assert_eq!(
        tool_call.function.arguments,
        rig_core::serde_json::json!({})
    );
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
    };

    let local_llm = LocalLlm::new(&config).expect("LocalLlm should build");

    let agent = AgentBuilder::new(local_llm.model())
        .tool(SystemStatusTool)
        .build();

    let assistant = Assistant::new(agent);

    let response = assistant
        .respond("Check the system status.")
        .await
        .expect("system status roundtrip should succeed");

    assert_eq!(response.output, "System status retrieved.");
    assert_eq!(response.requests(), 2);
}

#[tokio::test]
async fn openai_adapter_rejects_length_before_tool_execution() {
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
    };

    let local_llm = LocalLlm::new(&config).expect("LocalLlm should build");

    let agent = AgentBuilder::new(local_llm.model())
        .tool(SystemStatusTool)
        .build();

    let assistant = Assistant::new(agent);

    let result = assistant.respond("Check the system status.").await;

    assert!(
        result.is_err(),
        "length-terminated response must be rejected"
    );
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
    };

    let local_llm = LocalLlm::new(&config).expect("LocalLlm should build");

    let agent = AgentBuilder::new(local_llm.model())
        .tool(SystemStatusTool)
        .build();

    let assistant = Assistant::new(agent);

    let result = assistant.respond("Check the system status.").await;

    assert!(
        result.is_err(),
        "content-filtered response must be rejected"
    );
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
    };

    let local_llm = LocalLlm::new(&config).expect("LocalLlm should build");

    let agent = AgentBuilder::new(local_llm.model())
        .tool(SystemStatusTool)
        .build();

    let assistant = Assistant::new(agent);

    let result = assistant.respond("Check the system status.").await;

    assert!(result.is_err(), "unknown finish reason must be rejected");
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
    };

    let local_llm = LocalLlm::new(&config).expect("LocalLlm should build");

    // Only system_status is registered.
    let agent = AgentBuilder::new(local_llm.model())
        .tool(SystemStatusTool)
        .build();

    let assistant = Assistant::new(agent);

    let result = assistant.respond("Reboot the system.").await;

    assert!(result.is_err(), "unregistered tool must be rejected");
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
    };

    let local_llm = LocalLlm::new(&config).expect("LocalLlm should build");

    let agent = AgentBuilder::new(local_llm.model())
        .tool(SystemStatusTool)
        .build();

    let assistant = Assistant::new(agent);

    let result = assistant.respond("Check the system status.").await;

    assert!(
        result.is_err(),
        "invalid system_status arguments must be rejected"
    );
}
