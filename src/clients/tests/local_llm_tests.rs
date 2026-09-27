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
