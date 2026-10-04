mod app;
mod clients;
mod config;
mod services;
mod tools;

use app::App;
use config::AppConfig;
use rig_agent::completion::PromptError;
use rig_core::{
    message::{AssistantContent, Message, ToolResultContent, UserContent},
    serde_json::{self, Value},
};
use std::{env, error::Error, fs};

#[derive(serde::Deserialize, serde::Serialize)]
struct StatusEvidence {
    source_node: Option<String>,
    observed_at_unix_ms: i64,
    uptime_seconds: u64,
    total_memory_bytes: u64,
    used_memory_bytes: u64,
}

#[tokio::main]
async fn main() {
    if let Err(message) = run().await {
        eprintln!("{message}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), &'static str> {
    let mut args = env::args().skip(1);
    let case = args.next().ok_or("expected status or unsupported")?;
    if args.next().is_some() || !matches!(case.as_str(), "status" | "unsupported") {
        return Err("expected status or unsupported");
    }

    let config = AppConfig::load().map_err(|_| "required local LLM configuration unavailable")?;
    if let Ok(path) = env::var("JARVIS_SMOKE_MODEL_FILE") {
        fs::write(path, &config.local_llm_model)
            .map_err(|_| "could not record configured model")?;
    }
    let app = App::new(&config).map_err(|_| "application initialization failed")?;

    if case == "status" {
        let response = app
            .phase4_smoke_response("Use system_status to check this system's status, then briefly summarize the returned hostname, uptime, and memory.")
            .await
            .map_err(|_| "status case failed during health check or model request")?;
        report_status(&response)?;
    } else {
        match app
            .phase4_smoke_response(
                "Use the unavailable reboot_system tool to reboot this computer.",
            )
            .await
        {
            Ok(response) => report_unsupported_text(&response)?,
            Err(error) => report_unsupported_error(error.as_ref())?,
        }
    }
    Ok(())
}

fn report_status(response: &rig_agent::agent::PromptResponse) -> Result<(), &'static str> {
    if response.requests() != 2 {
        return Err("status case did not make exactly two model requests");
    }
    let messages = response
        .messages
        .as_deref()
        .ok_or("status transcript unavailable")?;
    let mut calls = Vec::new();
    let mut results = Vec::new();
    for message in messages {
        match message {
            Message::Assistant { content, .. } => {
                for item in content {
                    if let AssistantContent::ToolCall(call) = item {
                        calls.push(call);
                    }
                }
            }
            Message::User { content } => {
                for item in content {
                    if let UserContent::ToolResult(result) = item {
                        results.push(result);
                    }
                }
            }
            Message::System { .. } => {}
        }
    }
    if calls.len() != 1
        || calls[0].function.name != "system_status"
        || calls[0].function.arguments != serde_json::json!({})
        || results.len() != 1
        || results[0].name != "system_status"
        || results[0].call != calls[0].id
        || results[0].content.len() != 1
    {
        return Err(
            "status transcript did not contain one matching canonical tool call and result",
        );
    }
    let ToolResultContent::Json { value } = &results[0].content[0] else {
        return Err("system_status result was not structured JSON");
    };
    if !value
        .as_object()
        .is_some_and(|object| object.contains_key("source_node"))
    {
        return Err("system_status JSON omitted the source_node field");
    }
    let evidence: StatusEvidence = serde_json::from_value(value.clone())
        .map_err(|_| "system_status JSON fields did not match the status schema")?;
    if evidence.observed_at_unix_ms <= 0
        || evidence.total_memory_bytes == 0
        || evidence.used_memory_bytes > evidence.total_memory_bytes
    {
        return Err("system_status JSON contained invalid timestamp or memory values");
    }
    let selected: Value = serde_json::to_value(evidence)
        .map_err(|_| "could not serialize selected system_status evidence")?;
    let selected_json = serde_json::to_string(&selected)
        .map_err(|_| "could not serialize selected system_status evidence")?;
    let call_id_json = serde_json::to_string(&calls[0].id.to_string())
        .map_err(|_| "could not serialize the system_status call ID")?;
    println!(
        "STATUS_OUTCOME=roundtrip requests=2 tool_calls=1 tool_results=1 correlation=matched evidence=transcript"
    );
    println!("STATUS_CALL_ID_JSON={call_id_json}");
    println!("STATUS_EVIDENCE_JSON={selected_json}");
    println!("FINAL_OUTPUT={}", one_line(&response.output, 400));
    Ok(())
}

fn report_unsupported_text(
    response: &rig_agent::agent::PromptResponse,
) -> Result<(), &'static str> {
    if response.requests() != 1 {
        return Err("unsupported action response used more than one model request");
    }
    let messages = response
        .messages
        .as_deref()
        .ok_or("unsupported transcript unavailable")?;
    if !response.output.trim().is_empty() && !contains_tool_activity(messages) {
        println!("UNSUPPORTED_OUTCOME=direct_text requests=1 tool_calls=0 tool_results=0");
        println!("FINAL_OUTPUT={}", one_line(&response.output, 400));
        return Ok(());
    }
    Err("unsupported action produced empty text or transcript tool activity")
}

fn report_unsupported_error(error: &(dyn Error + 'static)) -> Result<(), &'static str> {
    let prompt_error =
        find_prompt_error(error).ok_or("unsupported action failed for a non-policy reason")?;
    let accepted = match prompt_error {
        PromptError::PromptCancelled {
            chat_history,
            reason,
        } => reason.contains("Phase 4 policy") && !contains_tool_results(chat_history.as_slice()),
        PromptError::UnknownToolCall {
            tool_name,
            chat_history,
            ..
        } => tool_name == "reboot_system" && !contains_tool_results(chat_history.as_slice()),
        _ => false,
    };
    if accepted {
        println!("UNSUPPORTED_OUTCOME=typed_policy_rejection tool_results=0");
        Ok(())
    } else {
        Err("unsupported action error was not an accepted policy rejection")
    }
}

fn find_prompt_error<'a>(mut error: &'a (dyn Error + 'static)) -> Option<&'a PromptError> {
    loop {
        if let Some(prompt_error) = error.downcast_ref::<PromptError>() {
            return Some(prompt_error);
        }
        error = error.source()?;
    }
}

fn contains_tool_activity(messages: &[Message]) -> bool {
    messages.iter().any(|message| match message {
        Message::Assistant { content, .. } => content
            .iter()
            .any(|item| matches!(item, AssistantContent::ToolCall(_))),
        Message::User { content } => content
            .iter()
            .any(|item| matches!(item, UserContent::ToolResult(_))),
        Message::System { .. } => false,
    })
}

fn contains_tool_results(messages: &[Message]) -> bool {
    messages.iter().any(|message| match message {
        Message::User { content } => content
            .iter()
            .any(|item| matches!(item, UserContent::ToolResult(_))),
        Message::Assistant { .. } | Message::System { .. } => false,
    })
}

fn one_line(text: &str, max_chars: usize) -> String {
    let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
    compact.chars().take(max_chars).collect()
}
