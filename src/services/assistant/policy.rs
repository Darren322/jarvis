use rig_core::{completion::FinishReason, message::AssistantContent, serde_json::json};

#[derive(Debug, PartialEq, Eq)]
pub enum PolicyRejection {
    InvalidContent,
    InvalidToolCount,
    InvalidToolName,
    InvalidToolArguments,
    InvalidFinishReason,
}

// Represents which stage of the agent run is currently being validated.
//
// `#[derive(...)]` asks Rust to automatically generate implementations
// of common traits for this enum:
// - Debug     -> allows debug printing with `{:?}`
// - Clone     -> allows explicitly cloning the value with `.clone()`
// - Copy      -> allows the value to be copied instead of moved
// - PartialEq -> allows equality comparisons with `==` and `!=`
// - Eq        -> marks the type as supporting full equality
//
// `derive` uses Rust's derive macro system, so we do not need to
// manually implement these traits ourselves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyTurn {
    // First model turn. The model may return final text or propose
    // the permitted system_status tool.
    Initial,

    // A tool has already executed. The model should now produce
    // the final response rather than request another tool.
    AfterTool,
}

fn validate_finish_reason(final_reason: Option<&FinishReason>) -> Result<(), PolicyRejection> {
    // A finish reason may be absent (`None`), so only check it when one exists.
    //
    // Rig's `truncated_output()` returns true when the model response was cut off
    // instead of completing normally (for example, reaching the output token limit).
    // Jarvis rejects truncated responses because the model's output may be incomplete
    // and should not be trusted for further tool execution.
    //
    // `is_some_and(...)` means:
    // Some(reason) -> run `truncated_output()` on the reason None         -> false
    if final_reason.is_some_and(FinishReason::truncated_output) {
        return Err(PolicyRejection::InvalidFinishReason);
    }

    // Result<(), E>: `()` means success has no value to return
    Ok(())
}

fn validate_finish_reason_consistency(
    contents: &[AssistantContent],
    finish_reason: Option<&FinishReason>,
) -> Result<(), PolicyRejection> {
    let has_tool_call = contents
        .iter()
        .any(|content| matches!(content, AssistantContent::ToolCall(_)));

    // No finish reason means there is nothing to compare against.
    // The other policy validators still validate the actual contents.
    let Some(finish_reason) = finish_reason else {
        return Ok(());
    };

    match finish_reason {
        FinishReason::ToolCalls if !has_tool_call => Err(PolicyRejection::InvalidFinishReason),

        FinishReason::Stop if has_tool_call => Err(PolicyRejection::InvalidFinishReason),

        _ => Ok(()),
    }
}

fn validate_tool_count(contents: &[AssistantContent]) -> Result<(), PolicyRejection> {
    // &[AssistantContent] = borrowed slice; roughly a read-only view over multiple
    // AssistantContent values without taking ownership of the collection.
    //
    // .iter() borrows each element.
    // .filter(...) keeps only elements matching the closure's condition.
    // |content| ... is Rust closure syntax, similar to a Java lambda.
    // matches! is a macro that checks whether a value matches an enum pattern.
    // ToolCall(_) means "ToolCall containing anything"; `_` ignores its inner value.
    let tool_count = contents
        .iter()
        .filter(|content| matches!(content, AssistantContent::ToolCall(_)))
        .count();

    if tool_count > 1 {
        return Err(PolicyRejection::InvalidToolCount);
    }

    Ok(())
}

fn validate_tool_name(contents: &[AssistantContent]) -> Result<(), PolicyRejection> {
    // `if let` is useful when we care about one enum variant.
    // If content is ToolCall(...), its inner value is bound to `tool_call`.
    // Other AssistantContent variants are ignored by this specific validator.
    for content in contents {
        if let AssistantContent::ToolCall(tool_call) = content {
            if tool_call.function.name != "system_status" {
                return Err(PolicyRejection::InvalidToolName);
            }
        }
    }

    Ok(())
}

fn validate_tool_arguments(contents: &[AssistantContent]) -> Result<(), PolicyRejection> {
    for content in contents {
        if let AssistantContent::ToolCall(tool_call) = content {
            // json! is a macro that constructs a serde_json::Value.
            // For Phase 4, system_status's canonical argument payload must be exactly {}.
            if tool_call.function.arguments != json!({}) {
                return Err(PolicyRejection::InvalidToolArguments);
            }
        }
    }

    Ok(())
}

fn validate_content(contents: &[AssistantContent]) -> Result<(), PolicyRejection> {
    let has_valid_content = contents.iter().any(|content| {
        matches! {
            content,
            AssistantContent::Text(_) | AssistantContent::ToolCall(_)
        }
    });

    // `matches!` is a Rust standard-library declarative macro.
    // It checks whether a value matches a given Rust pattern and returns a bool.
    //
    // Example:
    // matches!(content, AssistantContent::ToolCall(_))
    //   -> true  if `content` is ToolCall(...)
    //   -> false otherwise
    //
    // The `!` means `matches!` is a macro; it does NOT mean "not".
    // Internally, `matches!` is essentially shorthand for a `match` expression.

    if !has_valid_content {
        return Err(PolicyRejection::InvalidContent);
    }

    Ok(())
}

fn validate_policy_turn(
    turn: PolicyTurn,
    contents: &[AssistantContent],
) -> Result<(), PolicyRejection> {
    match turn {
        PolicyTurn::Initial => Ok(()),

        PolicyTurn::AfterTool => {
            let has_tool_call = contents
                .iter()
                .any(|content| matches!(content, AssistantContent::ToolCall(_)));

            if has_tool_call {
                return Err(PolicyRejection::InvalidContent);
            }

            Ok(())
        }
    }
}

// Combines the rules for an entire model run
fn validate_turn(
    turn: PolicyTurn,
    contents: &[AssistantContent],
    finish_reason: Option<&FinishReason>,
) -> Result<(), PolicyRejection> {
    validate_finish_reason(finish_reason)?;
    validate_finish_reason_consistency(contents, finish_reason)?;
    validate_content(contents)?;
    validate_tool_count(contents)?;
    validate_tool_name(contents)?;
    validate_tool_arguments(contents)?;
    validate_policy_turn(turn, contents)?;

    Ok(())
}

#[cfg(test)]
#[path = "tests/policy_tests.rs"]
mod policy_tests;
