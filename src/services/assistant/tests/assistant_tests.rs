use rig_agent::{
    AgentBuilder,
    prelude::{CompletionError, CompletionModel},
};
use rig_core::{
    completion::{CompletionRequest, CompletionResponse},
    streaming::StreamingCompletionResponse,
    test_utils::MockCompletionModel,
};

use crate::tools::system_status_tool::SystemStatusTool;

use super::*;
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

#[tokio::test]
async fn deadline_expires_for_pending_future() {
    let result = within_deadline(
        std::time::Duration::from_millis(10),
        std::future::pending::<()>(),
    )
    .await;

    assert!(result.is_err());
}

#[tokio::test]
async fn assistant_text_run() {
    let model = MockCompletionModel::text("JARVIS ONLINE");
    let agent = AgentBuilder::new(model).tool(SystemStatusTool).build();
    let assistant = Assistant::new(agent);

    let response = assistant
        .respond("Reply with exactly: JARVIS ONLINE")
        .await
        .expect("assistant should return a text response");

    assert_eq!(response.output, "JARVIS ONLINE");
}

#[tokio::test(start_paused = true)]
async fn assistant_run_times_out_when_model_never_responds() {
    let model = PendingCompletionModel;

    let agent = AgentBuilder::new(model).tool(SystemStatusTool).build();

    let assistant = Assistant::new(agent);

    let result = assistant.respond("Hello").await;

    assert!(
        matches!(result, Err(AssistantError::RunTimeout)),
        "hanging model should hit the Assistant's run deadline"
    );
}
