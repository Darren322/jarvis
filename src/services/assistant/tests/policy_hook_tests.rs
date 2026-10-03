use rig_agent::AgentBuilder;
use rig_core::test_utils::{MockCompletionModel, MockTurn};

use crate::{services::assistant::Assistant, tools::system_status_tool::SystemStatusTool};

use super::*;

#[tokio::test]
async fn allows_normal_text_response() {
    let model = MockCompletionModel::text("JARVIS ONLINE");

    let agent = AgentBuilder::new(model).tool(SystemStatusTool).build();

    let assistant = Assistant::new(agent);

    let response = assistant
        .respond("Reply with exactly: JARVIS ONLINE")
        .await
        .expect("valid text response should be accepted");

    assert_eq!(response.output, "JARVIS ONLINE");
}

#[tokio::test]
async fn rejects_unavailable_tool_call() {
    let model = MockCompletionModel::from_turns([MockTurn::tool_call(
        "call-1",
        "forbidden_tool",
        json!({}),
    )]);

    let agent = AgentBuilder::new(model).tool(SystemStatusTool).build();

    let assistant = Assistant::new(agent);

    let result = assistant.respond("Call the forbidden tool.").await;

    assert!(result.is_err(), "unavailable tool call should be rejected");
}
