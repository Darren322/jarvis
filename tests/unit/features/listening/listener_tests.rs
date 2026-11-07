use std::{fs, future::pending, os::unix::fs::PermissionsExt, time::Duration};

use serde_json::json;
use tempfile::TempDir;
use tokio::sync::oneshot;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

use super::*;
use crate::config::SpeechToTextConfig;

/// A recorder that runs `body` as a shell script. The directory must outlive it.
fn fake_recorder(body: &str) -> (TempDir, Recorder) {
    let dir = tempfile::tempdir().expect("temporary directory should open");
    let script = dir.path().join("fake-arecord");
    fs::write(&script, format!("#!/bin/sh\n{body}\n")).expect("script should write");
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).expect("script is runnable");
    let recorder = Recorder::new("test-device", script.to_str().expect("path is UTF-8"));
    (dir, recorder)
}

/// Emits `bytes` of silence and exits, so the recording ends by itself.
fn short_script(bytes: usize) -> String {
    format!("head -c {bytes} /dev/zero")
}

async fn server_answering(response: ResponseTemplate) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/audio/transcriptions"))
        .respond_with(response)
        .mount(&server)
        .await;
    server
}

fn text_reply(text: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({ "text": text }))
}

fn listener(recorder: Recorder, server: &MockServer) -> Listener {
    let config = SpeechToTextConfig {
        base_url: format!("{}/v1", server.uri()),
        model: "whisper-test".to_owned(),
        capture_device: "test-device".to_owned(),
    };
    Listener::new(
        recorder,
        SpeechToText::new(&config).expect("a localhost URL is allowed"),
    )
}

#[tokio::test]
async fn a_finished_recording_is_transcribed_after_announcing_the_stage() {
    let server = server_answering(text_reply("  what time is it  ")).await;
    let (_dir, recorder) = fake_recorder(&short_script(20_000));
    let (sender, mut announced) = oneshot::channel();

    let heard = listener(recorder, &server).listen(pending(), sender).await;

    assert_eq!(
        heard.expect("listening succeeds"),
        Heard::Text("what time is it".to_owned())
    );
    assert_eq!(announced.try_recv(), Ok(()));
    let requests = server.received_requests().await.expect("recording is on");
    assert_eq!(requests.len(), 1);
}

#[tokio::test]
async fn too_little_audio_is_not_sent_and_does_not_announce_transcribing() {
    let server = server_answering(text_reply("never asked")).await;
    let (_dir, recorder) = fake_recorder(&short_script(100));
    let (sender, mut announced) = oneshot::channel();

    let heard = listener(recorder, &server).listen(pending(), sender).await;

    assert_eq!(heard.expect("listening succeeds"), Heard::DidntCatch);
    assert_eq!(
        announced.try_recv(),
        Err(oneshot::error::TryRecvError::Closed)
    );
    let requests = server.received_requests().await.expect("recording is on");
    assert!(requests.is_empty());
}

#[tokio::test]
async fn a_transcript_with_no_words_is_didnt_catch_that() {
    let server = server_answering(text_reply("   ")).await;
    let (_dir, recorder) = fake_recorder(&short_script(20_000));
    let (sender, _announced) = oneshot::channel();

    let heard = listener(recorder, &server).listen(pending(), sender).await;

    assert_eq!(heard.expect("listening succeeds"), Heard::DidntCatch);
}

#[tokio::test]
async fn a_stop_that_is_already_signalled_ends_the_recording() {
    let server = server_answering(text_reply("never asked")).await;
    let (_dir, recorder) = fake_recorder("exec sleep 30");
    let (sender, _announced) = oneshot::channel();

    // Without the stop, the recorder's own 16 s deadline would be the only exit.
    let heard = tokio::time::timeout(
        Duration::from_secs(10),
        listener(recorder, &server).listen(std::future::ready(()), sender),
    )
    .await
    .expect("the stop signal must end the recording");

    assert_eq!(heard.expect("listening succeeds"), Heard::DidntCatch);
}

#[tokio::test]
async fn a_recorder_that_cannot_start_is_a_capture_error() {
    let server = server_answering(text_reply("never asked")).await;
    let recorder = Recorder::new("test-device", "/nonexistent/jarvis-arecord");
    let (sender, _announced) = oneshot::channel();

    let error = listener(recorder, &server)
        .listen(pending(), sender)
        .await
        .expect_err("the recorder is missing");

    assert!(
        matches!(error, ListenError::Capture(CaptureError::Spawn(_))),
        "{error:?}"
    );
}

#[tokio::test]
async fn a_server_failure_is_a_transcription_error() {
    let server = server_answering(ResponseTemplate::new(500)).await;
    let (_dir, recorder) = fake_recorder(&short_script(20_000));
    let (sender, _announced) = oneshot::channel();

    let error = listener(recorder, &server)
        .listen(pending(), sender)
        .await
        .expect_err("the server fails");

    assert!(
        matches!(
            error,
            ListenError::Transcription(SpeechToTextError::Server { .. })
        ),
        "{error:?}"
    );
}
