use rig_agent::{
    AgentBuilder,
    agent::PromptResponse,
    completion::PromptError,
    prelude::{CompletionError, CompletionModel},
};
use rig_core::{
    completion::{CompletionRequest, CompletionResponse, Usage},
    streaming::StreamingCompletionResponse,
    test_utils::{MockCompletionModel, MockTurn},
};
use std::time::Duration;

use crate::{
    services::assistant::{
        Assistant,
        error::AssistantError,
        run_observer::RunObserver,
        run_report::{CallUsage, ModelStage, RunOutcome},
    },
    tools::system_status_tool::SystemStatusTool,
};

#[derive(Clone)]
struct PendingCompletionModel;

impl CompletionModel for PendingCompletionModel {
    async fn completion(
        &self,
        _request: CompletionRequest,
    ) -> Result<CompletionResponse, CompletionError> {
        // Simulate an LLM request that never returns.
        std::future::pending().await
    }

    async fn stream(
        &self,
        _request: CompletionRequest,
    ) -> Result<StreamingCompletionResponse, CompletionError> {
        // Not used by Assistant::respond(), but required by CompletionModel.
        std::future::pending().await
    }
}

fn assert_incomplete_model(stage: &ModelStage, turn: usize) {
    assert_eq!(stage.turn, turn);
    assert!(!stage.completed);
    assert!(stage.elapsed.is_none());
    assert!(matches!(&stage.usage, CallUsage::Unavailable));
}

#[tokio::test]
async fn assistant_text_run_returns_success_report_without_tools() {
    let model = MockCompletionModel::from_turns([MockTurn::text("JARVIS ONLINE")]);
    let agent = AgentBuilder::new(model).tool(SystemStatusTool).build();
    let assistant = Assistant::new(agent);

    let run = assistant
        .respond("Reply with exactly: JARVIS ONLINE", &[])
        .await
        .expect("assistant should return a text response");

    assert_eq!(run.response.output, "JARVIS ONLINE");
    assert!(matches!(run.report.outcome, RunOutcome::Success));
    assert!(run.report.observations_available);
    assert!(run.report.tool_stages.is_empty());
    assert_eq!(run.report.model_stages.len(), 1);
    let stage = &run.report.model_stages[0];
    assert_eq!(stage.turn, 1);
    assert!(stage.completed);
    assert!(stage.elapsed.is_some());
    assert!(matches!(&stage.usage, CallUsage::Unavailable));
}

#[tokio::test]
async fn assistant_reports_usage_per_run_and_marks_missing_usage_unavailable() {
    let expected = Usage {
        input_tokens: 17,
        output_tokens: 9,
        total_tokens: 26,
        cached_input_tokens: 3,
        cache_creation_input_tokens: 4,
        tool_use_prompt_tokens: 5,
        reasoning_tokens: 6,
    };
    let model = MockCompletionModel::from_turns([
        MockTurn::text("usage reported").with_usage(expected),
        MockTurn::text("usage omitted").with_usage(Usage::new()),
    ]);
    let model_handle = model.clone();
    let assistant = Assistant::new(AgentBuilder::new(model).tool(SystemStatusTool).build());

    let first = assistant
        .respond("first run", &[])
        .await
        .expect("first run succeeds");
    let second = assistant
        .respond("independent second run", &[])
        .await
        .expect("second run succeeds");

    assert_eq!(model_handle.request_count(), 2);
    assert!(first.report.observations_available);
    assert!(second.report.observations_available);
    assert_eq!(first.report.model_stages.len(), 1);
    assert_eq!(second.report.model_stages.len(), 1);
    assert_eq!(first.report.model_stages[0].turn, 1);
    assert_eq!(second.report.model_stages[0].turn, 1);

    match &first.report.model_stages[0].usage {
        CallUsage::Normalized(actual) => assert_eq!(actual, &expected),
        CallUsage::Unavailable => panic!("reported usage must be preserved"),
    }
    assert!(matches!(
        &second.report.model_stages[0].usage,
        CallUsage::Unavailable
    ));
}

#[tokio::test(start_paused = true)]
async fn assistant_run_times_out_and_reports_incomplete_model_stage() {
    let agent = AgentBuilder::new(PendingCompletionModel)
        .tool(SystemStatusTool)
        .build();
    let assistant = Assistant::new(agent);

    let error = assistant
        .respond("Hello", &[])
        .await
        .expect_err("pending model should hit the Assistant deadline");

    assert!(matches!(&error.error, AssistantError::RunTimeout));
    assert!(matches!(&error.report.outcome, RunOutcome::TimedOut));
    assert!(error.report.observations_available);
    assert_eq!(error.report.model_stages.len(), 1);
    assert_incomplete_model(&error.report.model_stages[0], 1);
    assert!(error.report.tool_stages.is_empty());
}

#[tokio::test]
async fn provider_prompt_failure_reports_incomplete_model_stage_and_keeps_error_chain() {
    let model = MockCompletionModel::from_turns([MockTurn::error("provider failed")]);
    let model_handle = model.clone();
    let assistant = Assistant::new(AgentBuilder::new(model).tool(SystemStatusTool).build());

    let error = assistant
        .respond("Hello", &[])
        .await
        .expect_err("provider failure must be returned");

    assert!(matches!(&error.error, AssistantError::Prompt(_)));
    assert!(matches!(&error.report.outcome, RunOutcome::PromptFailed));
    assert!(error.report.observations_available);
    assert_eq!(error.report.model_stages.len(), 1);
    assert_incomplete_model(&error.report.model_stages[0], 1);

    let mut source: &(dyn std::error::Error + 'static) = error.as_ref();
    let prompt_error = loop {
        if let Some(prompt_error) = source.downcast_ref::<PromptError>() {
            break prompt_error;
        }
        source = source
            .source()
            .expect("prompt error source chain continues");
    };
    assert!(matches!(
        prompt_error,
        PromptError::CompletionError(CompletionError::ProviderError(message))
            if message == "provider failed"
    ));
    assert_eq!(model_handle.request_count(), 1);
}

#[test]
fn empty_final_response_is_classified_as_invalid_response() {
    let result = super::finalize_response(
        PromptResponse::new(" \n\t", Usage::new()),
        RunObserver::default(),
        Duration::ZERO,
    );

    let error = result.expect_err("blank final response must be rejected");
    assert!(matches!(&error.error, AssistantError::InvalidResponse));
    assert!(matches!(&error.report.outcome, RunOutcome::InvalidResponse));
}
