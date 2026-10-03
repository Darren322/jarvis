use rig_agent::AgentBuilder;
use rig_core::message::{AssistantContent, Message, ToolChoice, UserContent};
use rig_core::serde_json::json;
use rig_core::test_utils::{MockCompletionModel, MockTurn};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use crate::services::assistant::Assistant;
use rig_agent::tool::{Tool, ToolContext};

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct CountingSystemStatusArgs {}

/// Test-only status tool that records how many times its body executes.
#[derive(Debug, Clone)]
struct CountingSystemStatusTool {
    executions: Arc<AtomicUsize>,
    should_fail: bool,
}

/// Test-only error used to simulate a real tool execution failure.
#[derive(Debug)]
struct TestToolError;

impl std::fmt::Display for TestToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "test tool failure")
    }
}

impl std::error::Error for TestToolError {}

impl Tool for CountingSystemStatusTool {
    const NAME: &'static str = "system_status";

    type Args = CountingSystemStatusArgs;
    type Output = rig_core::serde_json::Value;
    type Error = TestToolError;

    fn description(&self) -> String {
        "Test system status tool".to_string()
    }

    fn parameters(&self) -> rig_core::serde_json::Value {
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        _args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        // Count the attempt even if the operation fails.
        self.executions.fetch_add(1, Ordering::SeqCst);

        // Lets tests simulate an execution failure after entering the tool body.
        if self.should_fail {
            return Err(TestToolError);
        }

        Ok(json!({
            "status": "ok"
        }))
    }
}

#[tokio::test]
async fn executes_system_status_once_then_returns_final_text() {
    let executions = Arc::new(AtomicUsize::new(0));

    let tool = CountingSystemStatusTool {
        executions: Arc::clone(&executions),
        should_fail: false,
    };

    // Request 1 proposes the tool; request 2 gives the final answer.
    let model = MockCompletionModel::from_turns([
        MockTurn::tool_call("call-1", "system_status", json!({})),
        MockTurn::text("System status retrieved."),
    ]);

    // Keep a handle so we can inspect what Rig sent to the model.
    let model_handle = model.clone();

    let agent = AgentBuilder::new(model).tool(tool).build();
    let assistant = Assistant::new(agent);

    let response = assistant
        .respond("Check the system status.")
        .await
        .expect("valid tool roundtrip should succeed");

    let requests = model_handle.requests();

    assert_eq!(response.output, "System status retrieved.");
    assert_eq!(executions.load(Ordering::SeqCst), 1);
    assert_eq!(model_handle.request_count(), 2);

    // First request may use system_status.
    assert_eq!(requests[0].tools.len(), 1);
    assert_eq!(requests[0].tools[0].name, "system_status");
    assert_eq!(requests[0].max_tokens, Some(1026));
    assert_eq!(requests[0].tool_choice, Some(ToolChoice::Auto));

    // Finalization request must not be allowed to execute tools.
    assert!(requests[1].tools.is_empty());
    assert_eq!(requests[1].tool_choice, Some(ToolChoice::None));

    // Find the original tool call stored in request 2's history.
    let Message::Assistant {
        content: assistant_content,
        ..
    } = &requests[1].chat_history[1]
    else {
        panic!("expected assistant tool-call message");
    };

    let AssistantContent::ToolCall(tool_call) = &assistant_content[0] else {
        panic!("expected tool call");
    };

    // Find the matching tool result Rig added to history.
    let Message::User {
        content: user_content,
    } = &requests[1].chat_history[2]
    else {
        panic!("expected user tool-result message");
    };

    let UserContent::ToolResult(tool_result) = &user_content[0] else {
        panic!("expected tool result");
    };

    // Tool result must refer to the same call ID.
    assert_eq!(tool_call.id, tool_result.call);
}

#[tokio::test]
async fn rejects_multiple_tool_calls_before_execution() {
    let executions = Arc::new(AtomicUsize::new(0));

    let tool = CountingSystemStatusTool {
        executions: Arc::clone(&executions),
        should_fail: false,
    };

    let model = MockCompletionModel::from_turns([MockTurn::from_contents([
        AssistantContent::tool_call("call-1", "system_status", json!({})),
        AssistantContent::tool_call("call-2", "system_status", json!({})),
    ])]);

    let model_handle = model.clone();

    let agent = AgentBuilder::new(model).tool(tool).build();
    let assistant = Assistant::new(agent);

    let result = assistant.respond("Check the system status twice.").await;

    assert!(result.is_err());

    assert_eq!(
        model_handle.request_count(),
        1,
        "invalid initial turn must stop without another model request"
    );

    // Invalid batch must be rejected before either tool executes.
    assert_eq!(
        executions.load(Ordering::SeqCst),
        0,
        "rejected model turn must not execute any tools"
    );
}

#[tokio::test]
async fn rejects_second_tool_call_after_execution() {
    let executions = Arc::new(AtomicUsize::new(0));

    let tool = CountingSystemStatusTool {
        executions: Arc::clone(&executions),
        should_fail: false,
    };

    let model = MockCompletionModel::from_turns([
        MockTurn::tool_call("call-1", "system_status", json!({})),
        MockTurn::tool_call("call-2", "system_status", json!({})),
    ]);

    let model_handle = model.clone();

    let agent = AgentBuilder::new(model).tool(tool).build();
    let assistant = Assistant::new(agent);

    let result = assistant.respond("Check the system status.").await;

    assert!(result.is_err());

    // Two model requests are allowed, but only one tool attempt.
    assert_eq!(model_handle.request_count(), 2);

    assert_eq!(
        executions.load(Ordering::SeqCst),
        1,
        "second tool call must be rejected before execution"
    );
}

#[tokio::test]
async fn rejects_invalid_arguments_before_execution() {
    for arguments in [
        json!(null),                 // null
        json!({"unexpected": true}), // object with forbidden field
        json!([]),                   // array
        json!("invalid"),            // string scalar
        json!(42),                   // number scalar
    ] {
        let executions = Arc::new(AtomicUsize::new(0));

        let tool = CountingSystemStatusTool {
            executions: Arc::clone(&executions),
            should_fail: false,
        };

        let model = MockCompletionModel::from_turns([MockTurn::tool_call(
            "call-1",
            "system_status",
            arguments,
        )]);

        let model_handle = model.clone();

        let agent = AgentBuilder::new(model).tool(tool).build();
        let assistant = Assistant::new(agent);

        let result = assistant.respond("Check the system status.").await;

        assert!(result.is_err());
        assert_eq!(
            model_handle.request_count(),
            1,
            "invalid arguments must stop after the initial model request"
        );
        // Bad arguments must never reach the tool body.
        assert_eq!(
            executions.load(Ordering::SeqCst),
            0,
            "invalid arguments must be rejected before tool execution"
        );
    }
}

#[tokio::test]
async fn rejects_unavailable_tool_call() {
    let model = MockCompletionModel::from_turns([MockTurn::tool_call(
        "call-1",
        "forbidden_tool",
        json!({}),
    )]);

    let model_handle = model.clone();

    let executions = Arc::new(AtomicUsize::new(0));

    let tool = CountingSystemStatusTool {
        executions: Arc::clone(&executions),
        should_fail: false,
    };

    let agent = AgentBuilder::new(model).tool(tool).build();
    let assistant = Assistant::new(agent);

    let result = assistant.respond("Call the forbidden tool.").await;

    assert!(result.is_err(), "unavailable tool call should be rejected");

    assert_eq!(
        model_handle.request_count(),
        1,
        "unknown tool must stop after the initial model request"
    );
    // An unregistered/disallowed tool must never execute our real tool body.
    assert_eq!(
        executions.load(Ordering::SeqCst),
        0,
        "unknown tool must be rejected before any tool execution"
    );
}

#[tokio::test]
async fn failed_tool_execution_continues_to_final_text() {
    let executions = Arc::new(AtomicUsize::new(0));

    // This time the tool deliberately fails after execution begins.
    let tool = CountingSystemStatusTool {
        executions: Arc::clone(&executions),
        should_fail: true,
    };

    // Rig should carry the failure into request 2 so the model can explain it.
    let model = MockCompletionModel::from_turns([
        MockTurn::tool_call("call-1", "system_status", json!({})),
        MockTurn::text("System status could not be retrieved."),
    ]);

    let model_handle = model.clone();

    let agent = AgentBuilder::new(model).tool(tool).build();
    let assistant = Assistant::new(agent);

    let response = assistant
        .respond("Check the system status.")
        .await
        .expect("tool failure should still allow final explanation");

    // The user still receives a useful final response.
    assert_eq!(response.output, "System status could not be retrieved.");

    // Failure still consumes the one permitted operation attempt.
    assert_eq!(
        executions.load(Ordering::SeqCst),
        1,
        "failed tool execution should still consume one attempt"
    );

    // Request 1 proposes the tool; request 2 explains the failure. No retry.
    assert_eq!(
        model_handle.request_count(),
        2,
        "tool failure should continue to exactly one final model request"
    );
    let requests = model_handle.requests();

    // Request 2 is finalization: no tool can execute again.
    assert!(requests[1].tools.is_empty());
    assert_eq!(requests[1].tool_choice, Some(ToolChoice::None));

    let Message::Assistant {
        content: assistant_content,
        ..
    } = &requests[1].chat_history[1]
    else {
        panic!("expected assistant tool-call message");
    };

    let AssistantContent::ToolCall(tool_call) = &assistant_content[0] else {
        panic!("expected tool call");
    };

    let Message::User {
        content: user_content,
    } = &requests[1].chat_history[2]
    else {
        panic!("expected failed tool-result message");
    };

    let UserContent::ToolResult(tool_result) = &user_content[0] else {
        panic!("expected tool result");
    };

    // Even though execution failed, the result must match the original call.
    assert_eq!(tool_call.id, tool_result.call);
}

#[tokio::test]
async fn direct_text_returns_without_tool_execution() {
    let executions = Arc::new(AtomicUsize::new(0));

    let tool = CountingSystemStatusTool {
        executions: Arc::clone(&executions),
        should_fail: false,
    };

    // Model answers directly instead of requesting a tool.
    let model = MockCompletionModel::from_turns([MockTurn::text("Hello from Jarvis.")]);

    let model_handle = model.clone();

    let agent = AgentBuilder::new(model).tool(tool).build();
    let assistant = Assistant::new(agent);

    let response = assistant
        .respond("Say hello.")
        .await
        .expect("direct text should succeed");

    assert_eq!(response.output, "Hello from Jarvis.");

    // Direct text should finish after one model request.
    assert_eq!(model_handle.request_count(), 1);

    // No tool was requested, so nothing should execute.
    assert_eq!(executions.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn rejects_empty_assistant_content() {
    let executions = Arc::new(AtomicUsize::new(0));

    let tool = CountingSystemStatusTool {
        executions: Arc::clone(&executions),
        should_fail: false,
    };

    // Model returns no usable assistant content.
    let model = MockCompletionModel::from_turns([MockTurn::from_contents([])]);

    let model_handle = model.clone();

    let agent = AgentBuilder::new(model).tool(tool).build();
    let assistant = Assistant::new(agent);

    let result = assistant.respond("Say something.").await;

    assert!(result.is_err(), "empty assistant content must be rejected");

    // Policy should reject immediately after the first model response.
    assert_eq!(
        model_handle.request_count(),
        1,
        "empty content must stop after the initial model request"
    );

    // No valid tool call existed, so nothing should execute.
    assert_eq!(
        executions.load(Ordering::SeqCst),
        0,
        "empty content must not execute any tool"
    );
}
