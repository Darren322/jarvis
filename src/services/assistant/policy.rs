use rig_core::{completion::FinishReason, message::AssistantContent, serde_json::json};

#[derive(Debug, PartialEq, Eq)]
pub(super) enum PolicyRejection {
    Content,
    ToolCount,
    ToolName,
    ToolArguments,
    FinishReason,
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
pub(super) enum PolicyTurn {
    // First model turn. The model may return final text or propose
    // the permitted system_status tool.
    Initial,

    // A tool has already executed. The model should now produce
    // the final response rather than request another tool.
    AfterTool,
}

fn validate_finish_reason(final_reason: Option<&FinishReason>) -> Result<(), PolicyRejection> {
    let Some(reason) = final_reason else {
        return Ok(());
    };

    match reason {
        FinishReason::Stop | FinishReason::ToolCalls => Ok(()),

        FinishReason::Length | FinishReason::ContentFilter | FinishReason::Other(_) => {
            Err(PolicyRejection::FinishReason)
        }
    }
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
        FinishReason::ToolCalls if !has_tool_call => Err(PolicyRejection::FinishReason),

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
        return Err(PolicyRejection::ToolCount);
    }

    Ok(())
}

fn validate_tool_name(contents: &[AssistantContent]) -> Result<(), PolicyRejection> {
    // `if let` is useful when we care about one enum variant.
    // If content is ToolCall(...), its inner value is bound to `tool_call`.
    // Other AssistantContent variants are ignored by this specific validator.
    for content in contents {
        if let AssistantContent::ToolCall(tool_call) = content
            && tool_call.function.name != "system_status"
        {
            return Err(PolicyRejection::ToolName);
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
                return Err(PolicyRejection::ToolArguments);
            }
        }
    }

    Ok(())
}

fn validate_content(contents: &[AssistantContent]) -> Result<(), PolicyRejection> {
    let has_valid_content = contents.iter().any(|content| match content {
        AssistantContent::Text(text) => !text.text.trim().is_empty(),
        AssistantContent::ToolCall(_) => true,
        _ => false,
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

    let has_leaked_tool_call = contents.iter().any(|content| {
        matches!(
            content,
            AssistantContent::Text(text)
                if text.text.trim_start()
                    .starts_with("<|tool_call>call:")
        )
    });

    if has_leaked_tool_call {
        return Err(PolicyRejection::Content);
    }
    if !has_valid_content {
        return Err(PolicyRejection::Content);
    }

    let has_image = contents
        .iter()
        .any(|content| matches!(content, AssistantContent::Image(_)));

    if has_image {
        return Err(PolicyRejection::Content);
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
                return Err(PolicyRejection::Content);
            }

            Ok(())
        }
    }
}

// Combines the rules for an entire model run
pub(super) fn validate_turn(
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
#[path = "../../../tests/unit/services/assistant/policy_tests.rs"]
mod policy_tests;
