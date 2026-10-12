use rig_core::serde_json::{Value, json};

use super::*;

fn text(value: &str) -> AssistantContent {
    AssistantContent::text(value)
}

fn call(name: &str, arguments: Value) -> AssistantContent {
    AssistantContent::tool_call("call-1", name, arguments)
}

fn status() -> AssistantContent {
    call("system_status", json!({}))
}

/// Each case sits on one side of a policy boundary: the last accepted value
/// and the first rejected one.
#[rustfmt::skip]
#[test]
fn every_policy_rule_is_checked_at_its_boundary() {
    use PolicyRejection as R;
    use PolicyTurn::{AfterTool, Initial};
    type Case = (&'static str, PolicyTurn, Vec<AssistantContent>, Option<FinishReason>, Result<(), R>);

    let cases: Vec<Case> = vec![
        // Content: one visible character is enough; nothing visible is not.
        ("one visible character",     Initial,   vec![text("a")],                    None, Ok(())),
        ("no content",                Initial,   vec![],                             None, Err(R::Content)),
        ("whitespace only",           Initial,   vec![text(" \n\t")],               None, Err(R::Content)),
        ("reasoning only",            Initial,   vec![AssistantContent::reasoning("thinking")], None, Err(R::Content)),
        ("image beside valid text",   Initial,   vec![text("ok"), AssistantContent::image_base64("AAAA", None, None)], None, Err(R::Content)),
        // Leaked tool-call markup is rejected only as a prefix.
        ("markup as prefix",          Initial,   vec![text("  <|tool_call>call:x{}")], None, Err(R::Content)),
        ("markup mid-sentence",       Initial,   vec![text("It said <|tool_call>call: once")], None, Ok(())),
        // Tool count: one call is the maximum.
        ("one tool call",             Initial,   vec![status()], Some(FinishReason::ToolCalls), Ok(())),
        ("two tool calls",            Initial,   vec![status(), status()],           None, Err(R::ToolCount)),
        // Tool name: exact match against the allowed list.
        ("unknown tool",              Initial,   vec![call("delete_everything", json!({}))], None, Err(R::ToolName)),
        ("allowed name, wrong case",  Initial,   vec![call("System_Status", json!({}))],     None, Err(R::ToolName)),
        // Arguments: exactly the tool's typed Args, as a JSON object.
        ("empty object",              Initial,   vec![call("system_status", json!({}))],        None, Ok(())),
        ("one extra field",           Initial,   vec![call("system_status", json!({"x": 1}))], None, Err(R::ToolArguments)),
        ("null",                      Initial,   vec![call("system_status", Value::Null)],      None, Err(R::ToolArguments)),
        ("empty array",               Initial,   vec![call("system_status", json!([]))],        None, Err(R::ToolArguments)),
        ("JSON text of an object",    Initial,   vec![call("system_status", json!("{}"))],      None, Err(R::ToolArguments)),
        // Turn: a tool may be proposed first, never after a tool ran.
        ("text after tool",           AfterTool, vec![text("Done.")], Some(FinishReason::Stop), Ok(())),
        ("tool after tool",           AfterTool, vec![status()],      None,                     Err(R::Content)),
        // Finish reason: absent or normal is fine; truncated or unknown is not.
        ("no finish reason",          Initial,   vec![text("ok")], None,                                  Ok(())),
        ("tool_calls without a call", Initial,   vec![text("ok")], Some(FinishReason::ToolCalls),         Err(R::FinishReason)),
        ("length",                    Initial,   vec![status()],   Some(FinishReason::Length),            Err(R::FinishReason)),
        ("content filter",            Initial,   vec![status()],   Some(FinishReason::ContentFilter),     Err(R::FinishReason)),
        ("provider-specific",         Initial,   vec![text("ok")], Some(FinishReason::Other("eos".into())), Err(R::FinishReason)),
    ];

    for (name, turn, contents, finish_reason, expected) in cases {
        assert_eq!(validate_turn(turn, &contents, finish_reason.as_ref()), expected, "{name}");
    }
}
