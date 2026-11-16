//! The compiled binary with the orb's WebSocket port: served when it is free, and
//! reported once (without stopping Jarvis) when it is not.

use std::{
    net::{Ipv4Addr, TcpListener},
    process::Stdio,
    time::Duration,
};

use futures_util::StreamExt;
use tokio::{
    io::AsyncWriteExt,
    process::{Child, Command},
    time::{sleep, timeout},
};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message, client::IntoClientRequest, http::HeaderValue},
};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

/// Real time: the binary starts as a separate process.
const STARTUP_BOUND: Duration = Duration::from_secs(30);

/// Starts Jarvis with its orb on `port`, its stdin open and its other servers
/// local mocks. Dropping the child kills it.
async fn start_jarvis(port: u16, directory: &tempfile::TempDir) -> (Child, MockServer, MockServer) {
    let llm = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&llm)
        .await;
    let embeddings = MockServer::start().await;

    let child = Command::new(env!("CARGO_BIN_EXE_jarvis"))
        .env_clear()
        .current_dir(directory.path())
        .env("LOCAL_LLM_BASE_URL", llm.uri() + "/v1")
        .env("LOCAL_LLM_HEALTH_URL", llm.uri() + "/health")
        .env("LOCAL_LLM_MODEL", "integration-model")
        .env(
            "JARVIS_ARCHIVE_PATH",
            directory.path().join("archive.sqlite3"),
        )
        .env("JARVIS_EMBEDDING_BASE_URL", embeddings.uri() + "/v1")
        .env("JARVIS_EMBEDDING_MODEL", "integration-embedding-model")
        .env("JARVIS_ORB_PORT", port.to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("Jarvis binary starts");
    (child, llm, embeddings)
}

/// A port nobody is listening on right now.
fn free_port() -> u16 {
    TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .expect("a free port exists")
        .local_addr()
        .expect("the bound address is known")
        .port()
}

#[tokio::test]
async fn the_binary_serves_the_orb_websocket_and_closes_it_when_jarvis_exits() {
    let port = free_port();
    let directory = tempfile::tempdir().expect("temporary integration directory opens");
    let (mut child, _llm, _embeddings) = start_jarvis(port, &directory).await;

    // The port opens a moment after the process starts.
    let url = format!("ws://127.0.0.1:{port}/");
    let mut socket = timeout(STARTUP_BOUND, async {
        loop {
            // The page is served from the default UI origin.
            let mut request = url.as_str().into_client_request().expect("a valid URL");
            request
                .headers_mut()
                .insert("Origin", HeaderValue::from_static("http://localhost:8766"));
            if let Ok((socket, _)) = connect_async(request).await {
                break socket;
            }
            sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("the orb port opens within its bound");

    let first = timeout(STARTUP_BOUND, socket.next())
        .await
        .expect("the first message arrives")
        .expect("the connection is open")
        .expect("the frame is valid");
    let Message::Text(text) = first else {
        panic!("the first frame is not text: {first:?}");
    };
    assert_eq!(text.as_str(), r#"{"type":"state","state":"idle"}"#);

    let mut stdin = child.stdin.take().expect("child stdin is piped");
    stdin
        .write_all(b"/exit\n")
        .await
        .expect("the exit command reaches the child");
    let status = timeout(STARTUP_BOUND, child.wait())
        .await
        .expect("Jarvis exits within its bound")
        .expect("Jarvis child process exits");
    assert!(status.success(), "Jarvis exit status: {status}");

    // The connection ends with Jarvis, whether by a close frame or by the socket.
    let end = timeout(STARTUP_BOUND, async {
        while let Some(Ok(Message::Text(_) | Message::Ping(_) | Message::Pong(_))) =
            socket.next().await
        {}
    })
    .await;
    assert!(
        end.is_ok(),
        "the WebSocket is still open after Jarvis exited"
    );
}

#[tokio::test]
async fn a_port_already_in_use_is_reported_once_and_jarvis_keeps_running() {
    let occupant = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("a port to occupy");
    let port = occupant
        .local_addr()
        .expect("the occupied address is known")
        .port();
    let directory = tempfile::tempdir().expect("temporary integration directory opens");
    let (mut child, _llm, _embeddings) = start_jarvis(port, &directory).await;

    let mut stdin = child.stdin.take().expect("child stdin is piped");
    stdin
        .write_all(b"/help\n/exit\n")
        .await
        .expect("commands reach the child");
    let output = timeout(STARTUP_BOUND, child.wait_with_output())
        .await
        .expect("Jarvis finishes within its bound")
        .expect("Jarvis child process exits");

    // Jarvis ran its commands: it carried on without the orb.
    assert!(
        output.status.success(),
        "Jarvis failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Commands:"), "help output: {stdout}");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        stderr.matches("orb screen is unavailable").count(),
        1,
        "warnings: {stderr}"
    );
    assert!(
        stderr.contains(&format!("127.0.0.1:{port}")),
        "the warning names the port: {stderr}"
    );
    drop(occupant);
}
