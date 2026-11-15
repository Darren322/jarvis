use std::time::Duration;

use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

use super::*;

async fn server_answering(response: ResponseTemplate) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(response)
        .mount(&server)
        .await;
    server
}

async fn check(server: &MockServer) -> HealthStatus {
    HealthCheck::new(&format!("{}/v1", server.uri()))
        .expect("a localhost URL is allowed")
        .check()
        .await
}

// These three use real time: under paused time the clock may jump past the
// timeout while the reply is still on its way over the socket.
#[tokio::test]
async fn a_health_check_that_returns_200_is_up() {
    let server = server_answering(ResponseTemplate::new(200)).await;
    assert_eq!(check(&server).await, HealthStatus::Up);
}

#[tokio::test]
async fn a_health_check_that_returns_500_is_down() {
    let server = server_answering(ResponseTemplate::new(500)).await;
    assert_eq!(check(&server).await, HealthStatus::Down);
}

#[tokio::test]
async fn a_health_check_that_cannot_connect_is_down() {
    // Nothing listens on port 1.
    let status = HealthCheck::new("http://127.0.0.1:1")
        .expect("a localhost URL is allowed")
        .check()
        .await;
    assert_eq!(status, HealthStatus::Down);
}

#[tokio::test(start_paused = true)]
async fn a_health_check_still_pending_at_the_timeout_is_down() {
    // The server's delay runs on its own real clock; this test's clock is paused
    // and jumps to the timeout.
    let server =
        server_answering(ResponseTemplate::new(200).set_delay(Duration::from_secs(30))).await;
    let started = tokio::time::Instant::now();
    assert_eq!(check(&server).await, HealthStatus::Down);
    assert_eq!(started.elapsed(), HEALTH_TIMEOUT);
}

#[test]
fn the_health_timeout_is_below_the_stats_interval() {
    assert!(HEALTH_TIMEOUT < crate::features::stats::STATS_INTERVAL);
}

#[test]
fn a_non_local_health_endpoint_is_refused() {
    assert!(matches!(
        HealthCheck::new("http://example.com/v1"),
        Err(HealthError::InvalidEndpoint(_))
    ));
}
