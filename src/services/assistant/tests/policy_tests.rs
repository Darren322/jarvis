use rig_core::serde_json::json;

use super::*;

#[test]
fn rejects_truncated_finish_reason() {
    let result = validate_finish_reason(Some(&FinishReason::Length));

    assert_eq!(result, Err(PolicyRejection::FinishReason));
}
#[test]
fn accepts_missing_finish_reason() {
    let result = validate_finish_reason(None);

    assert_eq!(result, Ok(()));
}
#[test]
fn accepts_normal_finish_reason() {
    let result = validate_finish_reason(Some(&FinishReason::Stop));

    assert_eq!(result, Ok(()));
}

#[test]
fn rejects_multiple_tool_calls() {
    let contents = vec![
        AssistantContent::tool_call("call_1", "system_status", json!({})),
        AssistantContent::tool_call("call_2", "system_status", json!({})),
    ];

    let result = validate_tool_count(&contents);

    assert_eq!(result, Err(PolicyRejection::ToolCount))
}

#[test]
fn accepts_system_status_tool_name() {
    let contents = vec![AssistantContent::tool_call(
        "call_1",
        "system_status",
        rig_core::serde_json::json!({}),
    )];

    let result = validate_tool_name(&contents);

    assert_eq!(result, Ok(()))
}

#[test]
fn rejects_invalid_tool_name() {
    let contents = vec![AssistantContent::tool_call(
        "call_1",
        "delete_everything",
        json!({}),
    )];

    let result = validate_tool_name(&contents);

    assert_eq!(result, Err(PolicyRejection::ToolName))
}

#[test]
fn rejects_non_empty_tools_arguments() {
    let contents = vec![AssistantContent::tool_call(
        "call_1",
        "system_status",
        json!({"foo":1}),
    )];

    let result = validate_tool_arguments(&contents);

    assert_eq!(result, Err(PolicyRejection::ToolArguments));
}

#[test]
fn rejects_null_tool_arguments() {
    let contents = vec![AssistantContent::tool_call(
        "call_1",
        "system_status",
        json!(()),
    )];

    let result = validate_tool_arguments(&contents);

    assert_eq!(result, Err(PolicyRejection::ToolArguments));
}

#[test]
fn rejects_empty_content() {
    let contents = vec![];

    let result = validate_content(&contents);

    assert_eq!(result, Err(PolicyRejection::Content));
}

#[test]
fn accepts_text_content() {
    let contents = vec![AssistantContent::text("JARVIS ONLINE")];

    let result = validate_content(&contents);

    assert_eq!(result, Ok(()));
}

#[test]
fn rejects_whitespace_only_text() {
    let contents = vec![AssistantContent::text("   ")];

    let result = validate_content(&contents);

    assert_eq!(result, Err(PolicyRejection::Content));
}

#[test]
fn accepts_tool_call_content() {
    let contents = vec![AssistantContent::tool_call(
        "call_1",
        "system_status",
        json!({}),
    )];

    let result = validate_content(&contents);

    assert_eq!(result, Ok(()));
}

#[test]
fn accepts_valid_system_status_turn() {
    let contents = vec![AssistantContent::tool_call(
        "call_1",
        "system_status",
        json!({}),
    )];

    let result = validate_turn(
        PolicyTurn::Initial,
        &contents,
        Some(&FinishReason::ToolCalls),
    );

    assert_eq!(result, Ok(()));
}

#[test]
fn rejects_tool_call_after_tool_execution() {
    let contents = vec![AssistantContent::tool_call(
        "call_1",
        "system_status",
        json!({}),
    )];

    let result = validate_turn(
        PolicyTurn::AfterTool,
        &contents,
        Some(&FinishReason::ToolCalls),
    );

    assert_eq!(result, Err(PolicyRejection::Content));
}

#[test]
fn accepts_final_text_after_tool_execution() {
    let contents = vec![AssistantContent::text(
        "System status collected successfully.",
    )];

    let result = validate_turn(PolicyTurn::AfterTool, &contents, Some(&FinishReason::Stop));

    assert_eq!(result, Ok(()));
}

#[test]
fn rejects_tool_calls_finish_reason_without_tool_call() {
    let contents = vec![AssistantContent::text("JARVIS ONLINE")];

    let result = validate_turn(
        PolicyTurn::Initial,
        &contents,
        Some(&FinishReason::ToolCalls),
    );

    assert_eq!(result, Err(PolicyRejection::FinishReason));
}

#[test]
fn accepts_text_and_tool_call_with_tool_calls_finish_reason() {
    let contents = vec![
        AssistantContent::text("I'll check the system status."),
        AssistantContent::tool_call("call_1", "system_status", json!({})),
    ];

    let result = validate_turn(
        PolicyTurn::Initial,
        &contents,
        Some(&FinishReason::ToolCalls),
    );

    assert_eq!(result, Ok(()));
}

#[test]
fn rejects_image_mixed_with_valid_text() {
    let contents = vec![
        AssistantContent::text("JARVIS ONLINE"),
        AssistantContent::image_base64("fake-image-data", None, None),
    ];

    let result = validate_content(&contents);

    assert_eq!(result, Err(PolicyRejection::Content));
}

#[test]
fn rejects_other_finish_reason() {
    let contents = vec![AssistantContent::text("JARVIS ONLINE")];

    let result = validate_turn(
        PolicyTurn::Initial,
        &contents,
        Some(&FinishReason::Other("provider_specific".to_string())),
    );

    assert_eq!(result, Err(PolicyRejection::FinishReason));
}
