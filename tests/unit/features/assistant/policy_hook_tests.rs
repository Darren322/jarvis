//! What the Rig hook adds on top of the pure policy (covered rule by rule in
//! `policy_tests.rs`): which tools each request offers, that a rejected turn
//! never reaches a tool, and the one-tool / two-turn limits end to end.

use rig_agent::AgentBuilder;
use rig_agent::tool::{Tool, ToolContext};
use rig_core::completion::{CompletionRequest, Usage};
use rig_core::message::{AssistantContent, Message, ToolChoice, UserContent};
use rig_core::serde_json::{Value, json};
use rig_core::test_utils::{MockCompletionModel, MockStreamEvent};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use crate::features::assistant::{
    Assistant, AssistantRun, AssistantRunError, run_report::ToolStageOutcome,
};

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct CountingArgs {}

/// Test-only `system_status` that counts how often its body runs.
#[derive(Clone)]
struct CountingStatusTool {
    executions: Arc<AtomicUsize>,
    should_fail: bool,
}

#[derive(Debug, thiserror::Error)]
#[error("test tool failure")]
struct TestToolError;

impl Tool for CountingStatusTool {
    const NAME: &'static str = "system_status";
    type Args = CountingArgs;
    type Output = Value;
    type Error = TestToolError;

    fn description(&self) -> String {
        "Test system status tool".to_owned()
    }

    fn parameters(&self) -> Value {
        json!({"type": "object", "properties": {}, "additionalProperties": false})
    }

    async fn call(&self, _: &mut ToolContext, _: Self::Args) -> Result<Value, TestToolError> {
        self.executions.fetch_add(1, Ordering::SeqCst);
        if self.should_fail {
            Err(TestToolError)
        } else {
            Ok(json!({"status": "ok"}))
        }
    }
}

struct Outcome {
    result: Result<AssistantRun, Box<AssistantRunError>>,
    executions: usize,
    requests: Vec<CompletionRequest>,
}

async fn run(turns: Vec<Vec<MockStreamEvent>>, should_fail: bool) -> Outcome {
    let executions = Arc::new(AtomicUsize::new(0));
    let tool = CountingStatusTool {
        executions: Arc::clone(&executions),
        should_fail,
    };
    let model = MockCompletionModel::from_stream_turns(turns);
    let handle = model.clone();
    let assistant = Assistant::new(AgentBuilder::new(model).tool(tool).build());
    let result = assistant.respond("Check the system status.", &[]).await;
    Outcome {
        result,
        executions: executions.load(Ordering::SeqCst),
        requests: handle.requests(),
    }
}

/// A streamed model turn made of the given events, ending with the final event.
fn turn(events: impl IntoIterator<Item = MockStreamEvent>) -> Vec<MockStreamEvent> {
    events
        .into_iter()
        .chain([MockStreamEvent::final_response(Usage::new())])
        .collect()
}

fn text(value: &str) -> Vec<MockStreamEvent> {
    turn([MockStreamEvent::text(value)])
}

fn status_call(id: &str) -> MockStreamEvent {
    MockStreamEvent::tool_call(id, "system_status", json!({}))
}

fn tool_call(id: &str, name: &str, arguments: Value) -> Vec<MockStreamEvent> {
    turn([MockStreamEvent::tool_call(id, name, arguments)])
}

/// Asserts the call in request 2's history is answered by a matching result.
fn assert_result_correlates_with_call(request: &CompletionRequest) {
    let Message::Assistant { content, .. } = &request.chat_history[1] else {
        panic!("expected the assistant tool-call message");
    };
    let call = content
        .iter()
        .find_map(|content| match content {
            AssistantContent::ToolCall(call) => Some(call),
            _ => None,
        })
        .expect("history keeps the tool call");
    let Message::User { content } = &request.chat_history[2] else {
        panic!("expected the tool-result message");
    };
    let UserContent::ToolResult(result) = &content[0] else {
        panic!("expected a tool result");
    };
    assert_eq!(call.id, result.call);
}

#[tokio::test]
async fn first_request_offers_allowed_tools_and_the_final_request_offers_none() {
    let outcome = run(
        vec![
            turn([MockStreamEvent::text("Checking."), status_call("call-1")]),
            text("All good."),
        ],
        false,
    )
    .await;

    let run = outcome.result.expect("an allowed tool roundtrip succeeds");
    assert_eq!(run.response.output, "All good.");
    assert_eq!(outcome.executions, 1);
    assert_eq!(run.report.tool_stages.len(), 1);
    assert!(matches!(
        run.report.tool_stages[0].outcome,
        ToolStageOutcome::Success
    ));

    let [first, second] = outcome.requests.as_slice() else {
        panic!("expected exactly two model requests");
    };
    let offered: Vec<_> = first.tools.iter().map(|tool| tool.name.as_str()).collect();
    assert_eq!(offered, ["system_status"]);
    assert_eq!(first.tool_choice, Some(ToolChoice::Auto));
    assert_eq!(first.max_tokens, Some(1024));
    assert!(second.tools.is_empty());
    assert_eq!(second.tool_choice, Some(ToolChoice::None));
    assert_result_correlates_with_call(second);
}

#[tokio::test]
async fn tool_count_boundary_zero_and_one_run_but_two_run_nothing() {
    let zero = run(vec![text("Hello.")], false).await;
    assert!(zero.result.is_ok());
    assert_eq!((zero.executions, zero.requests.len()), (0, 1));

    let one = run(
        vec![
            tool_call("call-1", "system_status", json!({})),
            text("Done."),
        ],
        false,
    )
    .await;
    assert!(one.result.is_ok());
    assert_eq!((one.executions, one.requests.len()), (1, 2));

    // A batch over the limit is rejected whole: not even the first call runs.
    let two = run(
        vec![turn([status_call("call-1"), status_call("call-2")])],
        false,
    )
    .await;
    assert!(two.result.is_err());
    assert_eq!((two.executions, two.requests.len()), (0, 1));
}

#[tokio::test]
async fn turn_boundary_a_tool_on_the_second_turn_is_rejected_after_one_execution() {
    let outcome = run(
        vec![
            tool_call("call-1", "system_status", json!({})),
            tool_call("call-2", "system_status", json!({})),
        ],
        false,
    )
    .await;

    assert!(outcome.result.is_err());
    assert_eq!((outcome.executions, outcome.requests.len()), (1, 2));
}

#[tokio::test]
async fn rejected_turns_stop_after_one_request_without_running_the_tool() {
    // One case per rejection path: Rig's invalid-tool-call hook (unknown
    // name) and Jarvis's model-turn hook (arguments, content).
    let cases = [
        (
            "unknown tool",
            tool_call("call-1", "forbidden_tool", json!({})),
        ),
        (
            "invalid arguments",
            tool_call("call-1", "system_status", json!({"x": 1})),
        ),
        ("empty content", turn([])),
    ];
    for (name, turn) in cases {
        let outcome = run(vec![turn], false).await;
        assert!(outcome.result.is_err(), "{name}");
        assert_eq!(
            (outcome.executions, outcome.requests.len()),
            (0, 1),
            "{name}"
        );
    }
}

#[tokio::test]
async fn a_failed_tool_consumes_its_attempt_and_still_gets_one_final_answer() {
    let outcome = run(
        vec![
            tool_call("call-1", "system_status", json!({})),
            text("Status could not be retrieved."),
        ],
        true,
    )
    .await;

    let run = outcome
        .result
        .expect("a tool failure is explained, not fatal");
    assert_eq!(run.response.output, "Status could not be retrieved.");
    assert!(matches!(
        run.report.tool_stages[0].outcome,
        ToolStageOutcome::Error
    ));
    assert_eq!((outcome.executions, outcome.requests.len()), (1, 2));
    assert!(outcome.requests[1].tools.is_empty());
    assert_result_correlates_with_call(&outcome.requests[1]);
}

/// Registered with Rig but deliberately absent from `ALLOWED_TOOLS`.
struct UnlistedTool;

impl Tool for UnlistedTool {
    const NAME: &'static str = "unlisted_tool";
    type Args = CountingArgs;
    type Output = Value;
    type Error = TestToolError;

    fn description(&self) -> String {
        "A tool the policy has not allowed".to_owned()
    }

    fn parameters(&self) -> Value {
        json!({"type": "object", "properties": {}, "additionalProperties": false})
    }

    async fn call(&self, _: &mut ToolContext, _: Self::Args) -> Result<Value, TestToolError> {
        panic!("an unlisted tool must never run");
    }
}

#[tokio::test]
async fn a_registered_tool_missing_from_the_allowed_list_is_never_offered_or_run() {
    let model = MockCompletionModel::from_stream_turns([turn([MockStreamEvent::tool_call(
        "call-1",
        "unlisted_tool",
        json!({}),
    )])]);
    let handle = model.clone();
    let agent = AgentBuilder::new(model)
        .tool(CountingStatusTool {
            executions: Arc::new(AtomicUsize::new(0)),
            should_fail: false,
        })
        .tool(UnlistedTool)
        .build();

    let result = Assistant::new(agent)
        .respond("Use the unlisted tool.", &[])
        .await;

    assert!(result.is_err());
    let requests = handle.requests();
    let offered: Vec<_> = requests[0]
        .tools
        .iter()
        .map(|tool| tool.name.as_str())
        .collect();
    assert_eq!(offered, ["system_status"]);
    assert_eq!(requests.len(), 1);
}
