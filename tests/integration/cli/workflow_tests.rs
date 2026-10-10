use std::{process::Stdio, time::Duration};

use tokio::{io::AsyncWriteExt, process::Command};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

#[tokio::test]
async fn binary_runs_help_reset_exit_with_local_health_and_no_model_request() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;

    let directory = tempfile::tempdir().expect("temporary integration directory opens");
    let archive_path = directory.path().join("archive.sqlite3");
    let missing_assets = directory.path().join("missing-model-assets");
    let index_path = directory.path().join("memory-index");

    let mut child = Command::new(env!("CARGO_BIN_EXE_jarvis"))
        .env_clear()
        .current_dir(directory.path())
        .env("LOCAL_LLM_BASE_URL", server.uri() + "/v1")
        .env("LOCAL_LLM_HEALTH_URL", server.uri() + "/health")
        .env("LOCAL_LLM_MODEL", "integration-model")
        .env("JARVIS_ARCHIVE_PATH", &archive_path)
        .env("JARVIS_EMBEDDING_MODEL_DIR", &missing_assets)
        .env("JARVIS_MEMORY_INDEX_DIR", &index_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("Jarvis binary starts");

    let mut stdin = child.stdin.take().expect("child stdin is piped");
    stdin
        .write_all(b"/help\n/reset\n/exit\n")
        .await
        .expect("workflow commands reach the child");
    drop(stdin);

    let output = tokio::time::timeout(Duration::from_secs(30), child.wait_with_output())
        .await
        .expect("Jarvis command workflow finishes within its bound")
        .expect("Jarvis child process exits");
    assert!(
        output.status.success(),
        "Jarvis failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Enter a message"),
        "startup guide: {stdout}"
    );
    assert!(stdout.contains("Commands:"), "help output: {stdout}");
    assert!(
        stdout.contains("Conversation context reset."),
        "reset output: {stdout}"
    );

    let requests = server
        .received_requests()
        .await
        .expect("local mock recorded requests");
    assert_eq!(
        requests
            .iter()
            .filter(|request| {
                request.method.as_str() == "GET" && request.url.path() == "/health"
            })
            .count(),
        1,
        "startup performs one local health check"
    );
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.method.as_str() == "POST")
            .count(),
        0,
        "help/reset/exit must not request model completion"
    );

    let connection = rusqlite::Connection::open(&archive_path).expect("temporary archive opens");
    let session_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))
        .expect("session rows are queryable");
    assert_eq!(
        session_count, 2,
        "startup and reset each create one session"
    );
}
