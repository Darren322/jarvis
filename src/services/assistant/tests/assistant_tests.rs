use rig_agent::AgentBuilder;
use rig_core::test_utils::MockCompletionModel;

use super::*;

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
    let agent = AgentBuilder::new(model).build();
    let assistant = Assistant::new(agent);

    let response = assistant
        .respond("Reply with exactly: JARVIS ONLINE")
        .await
        .expect("assistant should return a text response");

    assert_eq!(response.output, "JARVIS ONLINE");
}
