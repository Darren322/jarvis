use rig_agent::AgentBuilder;
use rig_core::message::AssistantContent;
use rig_core::serde_json::json;
use rig_core::test_utils::{MockCompletionModel, MockTurn};
use std::{
    convert::Infallible,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use crate::services::assistant::Assistant;
use rig_agent::tool::{Tool, ToolContext};

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct CountingSystemStatusArgs {}

#[derive(Debug, Clone)]
struct CountingSystemStatusTool {
    executions: Arc<AtomicUsize>,
}

impl Tool for CountingSystemStatusTool {
    const NAME: &'static str = "system_status";

    type Args = CountingSystemStatusArgs;
    type Output = rig_core::serde_json::Value;
    type Error = Infallible;

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
        self.executions.fetch_add(1, Ordering::SeqCst);

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
    };

    let model = MockCompletionModel::from_turns([
        MockTurn::tool_call("call-1", "system_status", json!({})),
        MockTurn::text("System status retrieved."),
    ]);

    let agent = AgentBuilder::new(model).tool(tool).build();

    let assistant = Assistant::new(agent);

    let response = assistant
        .respond("Check the system status.")
        .await
        .expect("valid tool roundtrip should succeed");

    assert_eq!(response.output, "System status retrieved.");
    assert_eq!(executions.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn rejects_multiple_tool_calls_before_execution() {
    let executions = Arc::new(AtomicUsize::new(0));

    let tool = CountingSystemStatusTool {
        executions: Arc::clone(&executions),
    };

    let model = MockCompletionModel::from_turns([MockTurn::from_contents([
        AssistantContent::tool_call("call-1", "system_status", json!({})),
        AssistantContent::tool_call("call-2", "system_status", json!({})),
    ])]);
    let agent = AgentBuilder::new(model).tool(tool).build();

    let assistant = Assistant::new(agent);

    let result = assistant.respond("Check the system status twice.").await;

    assert!(result.is_err());

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
    };

    let model = MockCompletionModel::from_turns([
        MockTurn::tool_call("call-1", "system_status", json!({})),
        MockTurn::tool_call("call-2", "system_status", json!({})),
    ]);

    let agent = AgentBuilder::new(model).tool(tool).build();

    let assistant = Assistant::new(agent);

    let result = assistant.respond("Check the system status.").await;

    assert!(result.is_err());

    assert_eq!(
        executions.load(Ordering::SeqCst),
        1,
        "second tool call must be rejected before execution"
    );
}

#[tokio::test]
async fn rejects_invalid_arguments_before_execution() {
    for arguments in [json!(null), json!({"unexpected": true})] {
        let executions = Arc::new(AtomicUsize::new(0));

        let tool = CountingSystemStatusTool {
            executions: Arc::clone(&executions),
        };

        let model = MockCompletionModel::from_turns([MockTurn::tool_call(
            "call-1",
            "system_status",
            arguments,
        )]);

        let agent = AgentBuilder::new(model).tool(tool).build();

        let assistant = Assistant::new(agent);

        let result = assistant.respond("Check the system status.").await;

        assert!(result.is_err());

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

    let executions = Arc::new(AtomicUsize::new(0));

    let tool = CountingSystemStatusTool {
        executions: Arc::clone(&executions),
    };

    let agent = AgentBuilder::new(model).tool(tool).build();
    let assistant = Assistant::new(agent);

    let result = assistant.respond("Call the forbidden tool.").await;

    assert!(result.is_err(), "unavailable tool call should be rejected");
    assert_eq!(
        executions.load(Ordering::SeqCst),
        0,
        "unknown tool must be rejected before any tool execution"
    );
}
