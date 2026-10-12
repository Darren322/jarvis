use super::*;
use futures_util::StreamExt;
use rig_agent::{
    AgentBuilder,
    agent::{
        MultiTurnStreamItem,
        hook::{
            AgentHook, HookContext, ToolCall, ToolCallAction, ToolResultAction, ToolResultEvent,
        },
    },
    tool::{Tool, ToolContext, ToolExecutionError},
};
use rig_core::{
    completion::{AssistantContent, Usage},
    message::{ToolCall as MessageToolCall, ToolFunction},
    serde_json::{self, json},
    test_utils::{MockCompletionModel, MockStreamEvent, MockTurn},
};
use std::{
    collections::{HashMap, HashSet},
    convert::Infallible,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, oneshot};

#[derive(Debug, serde::Deserialize)]
struct ProbeArgs {
    outcome: String,
    label: String,
}

#[derive(Clone)]
struct GatedProbeTool {
    started: mpsc::UnboundedSender<String>,
    gates: Arc<Mutex<HashMap<String, oneshot::Receiver<()>>>>,
    executed: Arc<Mutex<Vec<String>>>,
}

impl Tool for GatedProbeTool {
    const NAME: &'static str = "outcome_probe";

    type Args = ProbeArgs;
    type Output = String;
    type Error = ToolExecutionError;

    fn description(&self) -> String {
        "A deterministic tool used to exercise run observations".to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "outcome": { "type": "string" },
                "label": { "type": "string" }
            },
            "required": ["outcome", "label"],
            "additionalProperties": false
        })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        let _ = self.started.send(args.label.clone());
        let gate = match self.gates.lock() {
            Ok(mut gates) => gates.remove(&args.label),
            Err(_) => None,
        };
        if let Some(gate) = gate {
            let _ = gate.await;
        }
        if let Ok(mut executed) = self.executed.lock() {
            executed.push(args.label);
        }

        match args.outcome.as_str() {
            "error" => Err(ToolExecutionError::other("expected tool error")),
            "refused" => Err(ToolExecutionError::refused("expected tool refusal")),
            _ => Ok("tool complete".to_string()),
        }
    }
}

#[derive(Clone)]
struct SkipByProviderId;

impl AgentHook for SkipByProviderId {
    async fn on_tool_call(&self, _ctx: &HookContext, event: ToolCall<'_>) -> ToolCallAction {
        if event.tool_call_id == Some("provider-skip") {
            ToolCallAction::skip("test policy skipped this call")
        } else {
            ToolCallAction::Run
        }
    }
}

#[derive(Debug)]
struct ResultRecord {
    provider_call_id: Option<String>,
    internal_call_id: String,
    name: String,
}

#[derive(Clone)]
struct ResultRecorder(mpsc::UnboundedSender<ResultRecord>);

impl AgentHook for ResultRecorder {
    async fn on_tool_result(
        &self,
        _ctx: &HookContext,
        event: ToolResultEvent<'_>,
    ) -> ToolResultAction {
        let _ = self.0.send(ResultRecord {
            provider_call_id: event.tool_call_id.map(str::to_string),
            internal_call_id: event.internal_call_id.to_string(),
            name: event.tool_name.to_string(),
        });
        ToolResultAction::Keep
    }
}

fn probe_call(id: &str, outcome: &str, label: &str) -> AssistantContent {
    AssistantContent::ToolCall(MessageToolCall::from_wire(
        id,
        ToolFunction::new(
            GatedProbeTool::NAME.to_string(),
            json!({ "outcome": outcome, "label": label }),
        ),
    ))
}

fn populated_usage() -> Usage {
    Usage {
        input_tokens: 11,
        output_tokens: 7,
        total_tokens: 18,
        cached_input_tokens: 3,
        cache_creation_input_tokens: 2,
        tool_use_prompt_tokens: 5,
        reasoning_tokens: 4,
    }
}

#[tokio::test]
async fn observer_records_a_reasoning_only_stream_turn_once_at_the_common_hook() {
    let expected_usage = populated_usage();
    let model = MockCompletionModel::from_stream_turns([[
        MockStreamEvent::reasoning("reasoning-only content"),
        MockStreamEvent::final_response(expected_usage),
    ]]);
    let observer = RunObserver::default();
    let mut stream = AgentBuilder::new(model)
        .build()
        .runner("reasoning-only turn")
        .add_hook(observer.clone())
        .max_turns(1)
        .stream()
        .await;

    let mut saw_final_response = false;
    while let Some(item) = stream.next().await {
        if let MultiTurnStreamItem::FinalResponse(_) =
            item.expect("reasoning-only stream should finish")
        {
            saw_final_response = true;
        }
    }
    assert!(saw_final_response);

    let report = observer.finish(RunOutcome::Success, Duration::from_secs(1));
    assert_eq!(report.model_stages.len(), 1);
    assert_eq!(report.model_stages[0].turn, 1);
    assert!(report.model_stages[0].completed);
    assert!(report.model_stages[0].elapsed.is_some());
    assert_eq!(
        report.model_stages[0].usage,
        CallUsage::Normalized(expected_usage)
    );
}

#[tokio::test]
async fn observer_correlates_overlapping_tool_results_and_keeps_all_rig_usage() {
    let (started_tx, mut started_rx) = mpsc::unbounded_channel();
    let (slow_release, slow_wait) = oneshot::channel();
    let (fast_release, fast_wait) = oneshot::channel();
    let gates = Arc::new(Mutex::new(HashMap::from([
        ("slow".to_string(), slow_wait),
        ("fast".to_string(), fast_wait),
    ])));
    let executed = Arc::new(Mutex::new(Vec::new()));

    let first_usage = populated_usage();
    let model = MockCompletionModel::from_turns([
        MockTurn::from_contents([
            probe_call("provider-slow", "success", "slow"),
            probe_call("provider-fast", "success", "fast"),
            probe_call("provider-error", "error", "error"),
            probe_call("provider-refused", "refused", "refused"),
            probe_call("provider-skip", "success", "skip"),
        ])
        .with_usage(first_usage),
        MockTurn::text("complete").with_usage(Usage::new()),
    ]);
    let agent = AgentBuilder::new(model)
        .tool(GatedProbeTool {
            started: started_tx,
            gates,
            executed: Arc::clone(&executed),
        })
        // This hook deliberately precedes the observer. Rig short-circuits its
        // on_tool_call chain for the skipped call, but still sends on_tool_result.
        .add_hook(SkipByProviderId)
        .build();

    let observer = RunObserver::default();
    let (result_tx, mut result_rx) = mpsc::unbounded_channel();
    let run = agent
        .runner("run all probe calls")
        .add_hook(observer.clone())
        .add_hook(ResultRecorder(result_tx))
        .max_turns(2)
        .tool_concurrency(5)
        .run();
    let run = tokio::spawn(run);

    let mut started = HashSet::new();
    while !started.contains("slow") || !started.contains("fast") {
        started.insert(started_rx.recv().await.expect("probe should start"));
    }

    let _ = fast_release.send(());
    let mut results = Vec::new();
    while !results
        .iter()
        .any(|record: &ResultRecord| record.provider_call_id.as_deref() == Some("provider-fast"))
    {
        results.push(result_rx.recv().await.expect("fast result should arrive"));
    }
    // The slow call remains blocked until the fast call has reached the result
    // hook, so result completion order is deterministic without timers.
    let _ = slow_release.send(());
    while results.len() < 5 {
        results.push(
            result_rx
                .recv()
                .await
                .expect("all tool results should arrive"),
        );
    }

    let response = run
        .await
        .expect("Rig run task should finish")
        .expect("run succeeds");
    assert_eq!(response.output, "complete");
    assert!(
        results
            .iter()
            .position(|record| record.provider_call_id.as_deref() == Some("provider-fast"))
            < results
                .iter()
                .position(|record| record.provider_call_id.as_deref() == Some("provider-slow"))
    );

    let report = observer.finish(RunOutcome::Success, Duration::from_secs(1));
    assert!(report.observations_available);
    assert_eq!(report.model_stages.len(), 2);
    assert_eq!(report.model_stages[0].turn, 1);
    assert!(report.model_stages[0].completed);
    assert!(report.model_stages[0].elapsed.is_some());
    assert_eq!(
        report.model_stages[0].usage,
        CallUsage::Normalized(first_usage)
    );
    assert_eq!(report.model_stages[1].turn, 2);
    assert!(report.model_stages[1].completed);
    assert!(report.model_stages[1].elapsed.is_some());
    assert_eq!(report.model_stages[1].usage, CallUsage::Unavailable);

    let result_ids: HashSet<_> = results
        .iter()
        .map(|record| record.internal_call_id.clone())
        .collect();
    let stage_ids: HashSet<_> = report
        .tool_stages
        .iter()
        .map(|stage| stage.internal_call_id.clone())
        .collect();
    assert_eq!(result_ids.len(), 5);
    assert_eq!(stage_ids, result_ids);
    assert!(results.iter().all(|record| {
        record.provider_call_id.as_deref() != Some(record.internal_call_id.as_str())
    }));
    assert!(report.tool_stages.windows(2).all(|pair| {
        (pair[0].turn, &pair[0].internal_call_id) <= (pair[1].turn, &pair[1].internal_call_id)
    }));

    for record in &results {
        let stage = report
            .tool_stages
            .iter()
            .find(|stage| stage.internal_call_id == record.internal_call_id)
            .expect("every result should correlate with its Rig call id");
        assert_eq!(stage.turn, 1);
        assert_eq!(stage.name, record.name);
        match record.provider_call_id.as_deref() {
            Some("provider-error") => assert_eq!(stage.outcome, ToolStageOutcome::Error),
            Some("provider-refused") => assert_eq!(stage.outcome, ToolStageOutcome::Refused),
            Some("provider-skip") => {
                assert_eq!(stage.outcome, ToolStageOutcome::Skipped);
                assert_eq!(stage.elapsed, None);
            }
            Some("provider-fast" | "provider-slow") => {
                assert_eq!(stage.outcome, ToolStageOutcome::Success);
                assert!(stage.elapsed.is_some());
            }
            other => panic!("unexpected provider call id: {other:?}"),
        }
    }

    let executed = executed.lock().expect("execution record should be healthy");
    assert_eq!(executed.len(), 4);
    assert!(!executed.iter().any(|label| label == "skip"));
}

#[test]
fn finish_marks_in_flight_stages_incomplete_and_sorts_deterministically() {
    let observer = RunObserver::default();
    let started_at = Instant::now();
    if let Ok(mut observed) = observer.observed.lock() {
        // Seed distinct starts and finish call-b before call-a. This proves
        // correlation uses each Rig id's own start rather than one shared
        // timestamp or the completion order.
        observed.start_tool(
            1,
            "call-a".to_string(),
            "alpha_call".to_string(),
            started_at,
        );
        observed.start_tool(
            1,
            "call-b".to_string(),
            "beta_call".to_string(),
            started_at + Duration::from_secs(10),
        );
        observed.finish_tool(
            1,
            "call-b".to_string(),
            "ignored".to_string(),
            ToolStageOutcome::Success,
            started_at + Duration::from_secs(15),
        );
        observed.finish_tool(
            1,
            "call-a".to_string(),
            "ignored".to_string(),
            ToolStageOutcome::Success,
            started_at + Duration::from_secs(21),
        );
        observed.model_starts.insert(4, started_at);
        observed.model_starts.insert(2, started_at);
        observed.model_stages.push(ModelStage {
            turn: 3,
            completed: true,
            elapsed: Some(Duration::from_millis(1)),
            usage: CallUsage::Unavailable,
        });
        observed.start_tool(2, "tool-z".to_string(), "zeta".to_string(), started_at);
        observed.start_tool(2, "tool-a".to_string(), "alpha".to_string(), started_at);
        observed.tool_stages.push(ToolStage {
            turn: 1,
            internal_call_id: "finished".to_string(),
            name: "already_done".to_string(),
            outcome: ToolStageOutcome::Success,
            elapsed: Some(Duration::from_millis(2)),
        });
    }

    let report = observer.finish(RunOutcome::PromptFailed, Duration::from_secs(1));
    assert!(report.observations_available);
    assert_eq!(
        report
            .model_stages
            .iter()
            .map(|stage| stage.turn)
            .collect::<Vec<_>>(),
        vec![2, 3, 4]
    );
    assert!(!report.model_stages[0].completed);
    assert_eq!(report.model_stages[0].elapsed, None);
    assert_eq!(report.model_stages[0].usage, CallUsage::Unavailable);
    assert!(report.model_stages[1].completed);
    assert!(!report.model_stages[2].completed);
    assert_eq!(report.model_stages[2].elapsed, None);
    assert_eq!(report.model_stages[2].usage, CallUsage::Unavailable);

    assert_eq!(
        report
            .tool_stages
            .iter()
            .map(|stage| (stage.turn, stage.internal_call_id.as_str()))
            .collect::<Vec<_>>(),
        vec![
            (1, "call-a"),
            (1, "call-b"),
            (1, "finished"),
            (2, "tool-a"),
            (2, "tool-z")
        ]
    );
    assert_eq!(report.tool_stages[0].outcome, ToolStageOutcome::Success);
    assert_eq!(report.tool_stages[0].elapsed, Some(Duration::from_secs(21)));
    assert_eq!(report.tool_stages[1].outcome, ToolStageOutcome::Success);
    assert_eq!(report.tool_stages[1].elapsed, Some(Duration::from_secs(5)));
    assert_eq!(report.tool_stages[2].outcome, ToolStageOutcome::Success);
    for stage in &report.tool_stages[3..] {
        assert_eq!(stage.outcome, ToolStageOutcome::Incomplete);
        assert_eq!(stage.elapsed, None);
    }
}

#[derive(Clone)]
struct PendingTool(mpsc::UnboundedSender<()>);

impl Tool for PendingTool {
    const NAME: &'static str = "pending_tool";

    type Args = serde_json::Value;
    type Output = String;
    type Error = Infallible;

    fn description(&self) -> String {
        "A tool that waits until its run is cancelled".to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        json!({ "type": "object", "properties": {}, "additionalProperties": false })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        _args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        let _ = self.0.send(());
        std::future::pending().await
    }
}

#[tokio::test]
async fn cancelled_rig_run_finalizes_a_pending_tool_as_incomplete() {
    let model = MockCompletionModel::new([MockTurn::tool_call(
        "provider-pending",
        PendingTool::NAME,
        json!({}),
    )]);
    let (started_tx, mut started_rx) = mpsc::unbounded_channel();
    let agent = AgentBuilder::new(model)
        .tool(PendingTool(started_tx))
        .build();
    let observer = RunObserver::default();
    let run = agent
        .runner("wait for tool")
        .add_hook(observer.clone())
        .tool_concurrency(1)
        .run();
    let run = tokio::spawn(run);

    started_rx.recv().await.expect("pending tool should start");
    run.abort();
    let _ = run.await;

    let report = observer.finish(RunOutcome::TimedOut, Duration::from_secs(1));
    assert!(report.observations_available);
    assert_eq!(report.tool_stages.len(), 1);
    assert_eq!(report.tool_stages[0].turn, 1);
    assert!(!report.tool_stages[0].internal_call_id.is_empty());
    assert_eq!(report.tool_stages[0].name, PendingTool::NAME);
    assert_eq!(report.tool_stages[0].outcome, ToolStageOutcome::Incomplete);
    assert_eq!(report.tool_stages[0].elapsed, None);
}

#[tokio::test]
async fn poisoned_observation_state_is_discarded_and_callbacks_stay_passive() {
    let observer = RunObserver::default();
    let poisoned_state = Arc::clone(&observer.observed);
    let _ = std::thread::spawn(move || {
        let _guard = poisoned_state
            .lock()
            .expect("initial state should be healthy");
        panic!("poison observation state for test");
    })
    .join();

    let response = AgentBuilder::new(MockCompletionModel::text("passive observer"))
        .build()
        .runner("reply")
        .add_hook(observer.clone())
        .run()
        .await
        .expect("poisoned observations must not steer or panic in the run");
    assert_eq!(response.output, "passive observer");

    let report = observer.finish(RunOutcome::Success, Duration::from_secs(1));
    assert!(!report.observations_available);
    assert!(report.model_stages.is_empty());
    assert!(report.tool_stages.is_empty());
}
