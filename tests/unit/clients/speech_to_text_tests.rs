use std::time::Duration;

use serde_json::json;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

use super::*;

fn config(base_url: String) -> SpeechToTextConfig {
    SpeechToTextConfig {
        base_url,
        model: "whisper-test".to_owned(),
        capture_device: "plughw:3,0".to_owned(),
    }
}

fn client(server: &MockServer) -> SpeechToText {
    SpeechToText::new(&config(format!("{}/v1", server.uri()))).expect("a localhost URL is allowed")
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

/// Four samples: 1, 2, 3 and -2 as little-endian i16.
const PCM: [u8; 8] = [1, 0, 2, 0, 3, 0, 0xfe, 0xff];

async fn transcribe(server: &MockServer) -> Result<Transcript, SpeechToTextError> {
    client(server).transcribe(&PCM).await
}

fn find(haystack: &[u8], needle: &[u8]) -> usize {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
        .unwrap_or_else(|| panic!("{:?} not in body", String::from_utf8_lossy(needle)))
}

#[tokio::test]
async fn sends_a_multipart_wav_with_the_model_and_language_and_no_prompt() {
    let server = server_answering(text_reply("hello")).await;

    transcribe(&server).await.expect("transcription succeeds");

    let requests = server.received_requests().await.expect("recording is on");
    let [request] = requests.as_slice() else {
        panic!("expected one request, got {}", requests.len());
    };
    assert_eq!(request.url.path(), "/v1/audio/transcriptions");
    let content_type = request
        .headers
        .get("content-type")
        .expect("content type is set")
        .to_str()
        .expect("content type is text");
    assert!(
        content_type.starts_with("multipart/form-data"),
        "{content_type}"
    );

    let body = request.body.as_slice();
    let text = String::from_utf8_lossy(body);
    assert!(
        text.contains("name=\"model\"\r\n\r\nwhisper-test\r\n"),
        "{text}"
    );
    assert!(text.contains("name=\"language\"\r\n\r\nen\r\n"), "{text}");
    assert!(!text.contains("name=\"prompt\""), "{text}");
    assert!(
        text.contains("name=\"file\"; filename=\"audio.wav\""),
        "{text}"
    );

    // The WAV is the whole file part: from "RIFF" to the closing boundary.
    let start = find(body, b"RIFF");
    let wav = &body[start..];
    let end = find(wav, b"\r\n--");
    let wav = &wav[..end];
    assert_eq!(&wav[8..16], b"WAVEfmt ");
    assert_eq!(u16::from_le_bytes([wav[20], wav[21]]), 1, "PCM format");
    assert_eq!(u16::from_le_bytes([wav[22], wav[23]]), 1, "mono");
    assert_eq!(
        u32::from_le_bytes([wav[24], wav[25], wav[26], wav[27]]),
        16_000
    );
    assert_eq!(
        u16::from_le_bytes([wav[34], wav[35]]),
        16,
        "bits per sample"
    );
    assert_eq!(&wav[36..40], b"data");
    assert_eq!(
        u32::from_le_bytes([wav[40], wav[41], wav[42], wav[43]]) as usize,
        PCM.len()
    );
    assert_eq!(&wav[44..], &PCM);
}

#[tokio::test]
async fn a_trailing_odd_byte_is_not_a_whole_sample_and_is_dropped() {
    let wav = wav_from_pcm(&[1, 0, 2, 0, 9]).expect("encodes");

    assert_eq!(&wav[44..], &[1, 0, 2, 0]);
}

/// (reply text, expected outcome)
#[tokio::test]
async fn the_transcript_is_trimmed_and_blank_text_means_nothing_was_heard() {
    let cases = [
        (
            "  hello jarvis \n",
            Transcript::Text("hello jarvis".to_owned()),
        ),
        ("", Transcript::Nothing),
        (" \t\r\n ", Transcript::Nothing),
    ];

    for (reply, expected) in cases {
        let server = server_answering(text_reply(reply)).await;

        let outcome = transcribe(&server).await.expect("transcription succeeds");

        assert_eq!(outcome, expected, "reply {reply:?}");
    }
}

#[tokio::test]
async fn a_transcript_may_be_at_most_2048_bytes() {
    let cases = [
        (MAX_TRANSCRIPT_BYTES, true),
        (MAX_TRANSCRIPT_BYTES + 1, false),
    ];

    for (bytes, accepted) in cases {
        // Padding around the text proves the limit applies after trimming.
        let reply = format!("  {}  ", "a".repeat(bytes));
        let server = server_answering(text_reply(&reply)).await;

        let outcome = transcribe(&server).await;

        match (accepted, outcome) {
            (true, Ok(Transcript::Text(text))) => assert_eq!(text.len(), bytes),
            (false, Err(SpeechToTextError::TranscriptTooLong { bytes: found })) => {
                assert_eq!(found, bytes);
            }
            (_, other) => panic!("{bytes} bytes gave {other:?}"),
        }
    }
}

#[tokio::test]
async fn an_error_status_carries_the_servers_message() {
    let envelope =
        json!({"error": {"message": "audio too short", "type": "invalid_request_error"}});
    let server = server_answering(ResponseTemplate::new(400).set_body_json(envelope)).await;

    let error = transcribe(&server).await.expect_err("400 is an error");

    match error {
        SpeechToTextError::Server {
            status, message, ..
        } => {
            assert_eq!(status, StatusCode::BAD_REQUEST);
            assert_eq!(message, "audio too short");
        }
        other => panic!("expected a server error, got {other:?}"),
    }
}

#[tokio::test]
async fn an_error_status_without_an_envelope_carries_a_capped_raw_body() {
    let body = "x".repeat(MAX_SERVER_MESSAGE_CHARS + 1);
    let server = server_answering(ResponseTemplate::new(503).set_body_string(body)).await;

    let error = transcribe(&server).await.expect_err("503 is an error");

    match error {
        SpeechToTextError::Server {
            status, message, ..
        } => {
            assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
            assert_eq!(message.len(), MAX_SERVER_MESSAGE_CHARS);
        }
        other => panic!("expected a server error, got {other:?}"),
    }
}

#[tokio::test]
async fn malformed_or_incomplete_success_bodies_are_invalid_responses() {
    let bodies = ["not json", "{}", r#"{"text": 5}"#];

    for body in bodies {
        let server = server_answering(ResponseTemplate::new(200).set_body_string(body)).await;

        let error = transcribe(&server).await.expect_err("body is unusable");

        assert!(
            matches!(error, SpeechToTextError::InvalidResponse(_)),
            "body {body:?} gave {error:?}"
        );
    }
}

#[tokio::test]
async fn a_server_that_is_not_listening_is_a_connection_error() {
    // Bind to get a free port, then close it. Wiremock pools its servers, so
    // dropping one does not guarantee the port is closed.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a free port");
    let address = listener.local_addr().expect("bound address");
    drop(listener);
    let speech_to_text =
        SpeechToText::new(&config(format!("http://{address}/v1"))).expect("a local URL");

    let error = speech_to_text
        .transcribe(&PCM)
        .await
        .expect_err("nothing is listening");

    assert!(
        matches!(error, SpeechToTextError::Connection(_)),
        "{error:?}"
    );
}

/// Paused time drives reqwest's timer: the 20 s limit passes without waiting.
#[tokio::test(start_paused = true)]
async fn a_reply_slower_than_20_seconds_is_a_timeout() {
    assert_eq!(TRANSCRIPTION_TIMEOUT, Duration::from_secs(20));
    let server = server_answering(
        text_reply("late").set_delay(TRANSCRIPTION_TIMEOUT + Duration::from_secs(1)),
    )
    .await;

    let error = transcribe(&server).await.expect_err("the reply is late");

    assert!(matches!(error, SpeechToTextError::TimedOut(_)), "{error:?}");
}

#[tokio::test]
async fn the_health_probe_reads_the_server_root_not_the_v1_path() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;
    let speech_to_text = client(&server);

    speech_to_text.health_check().await.expect("healthy");
    server.verify().await;

    server.reset().await;
    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;
    let error = speech_to_text.health_check().await.expect_err("503");
    assert!(matches!(error, SpeechToTextError::Health(_)), "{error:?}");
}

#[tokio::test]
async fn redirects_are_not_followed() {
    let server = server_answering(
        ResponseTemplate::new(302).insert_header("location", "http://127.0.0.1:1/elsewhere"),
    )
    .await;

    let error = transcribe(&server).await.expect_err("302 is an error");

    assert!(
        matches!(error, SpeechToTextError::Server { status, .. } if status == StatusCode::FOUND),
        "{error:?}"
    );
}

#[test]
fn the_endpoint_guard_rejects_a_non_local_base_url() {
    let cases = [
        (
            "https://api.example.com/v1",
            LocalEndpointError::NonLocalHost,
        ),
        ("ftp://127.0.0.1/v1", LocalEndpointError::UnsupportedScheme),
        (
            "http://user:pass@127.0.0.1/v1",
            LocalEndpointError::CredentialsNotAllowed,
        ),
        ("not a url", LocalEndpointError::InvalidUrl),
    ];

    for (url, expected) in cases {
        match SpeechToText::new(&config(url.to_owned())) {
            Err(SpeechToTextError::InvalidEndpoint(found)) => assert_eq!(found, expected, "{url}"),
            Err(other) => panic!("{url} gave {other:?}"),
            Ok(_) => panic!("{url} was accepted"),
        }
    }
}

#[tokio::test]
async fn an_error_envelope_on_a_success_status_is_still_the_servers_error() {
    let envelope = json!({"error": {"message": "model not loaded"}});
    let server = server_answering(ResponseTemplate::new(200).set_body_json(envelope)).await;

    let error = transcribe(&server)
        .await
        .expect_err("an envelope is an error");

    assert!(
        matches!(&error, SpeechToTextError::Server { message, .. } if message == "model not loaded"),
        "{error:?}"
    );
}
