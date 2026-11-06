use super::super::sample_rate::{MAX_SAMPLE_RATE_HZ, MIN_SAMPLE_RATE_HZ};
use super::*;
use std::path::PathBuf;
use tokio::io::{AsyncWriteExt, duplex};

const PIPER_VOICE: &str = "en_GB-northern_english_male-medium";
const PID: u32 = 4242;

fn rate(hz: u32) -> SampleRate {
    SampleRate::new(hz).expect("test rate is in range")
}

fn supertonic_config() -> TtsConfig {
    TtsConfig {
        python: PathBuf::from("/venv/bin/python"),
        worker_script: PathBuf::from("/jarvis/scripts/tts/supertonic_worker.py"),
        engine: TtsEngine::Supertonic {
            model_dir: PathBuf::from("/models/supertonic"),
        },
        threads: 2,
        audio_device: None,
    }
}

fn piper_config() -> TtsConfig {
    TtsConfig {
        python: PathBuf::from("/venv/bin/python"),
        worker_script: PathBuf::from("/jarvis/scripts/tts/piper_worker.py"),
        engine: TtsEngine::Piper {
            model: PathBuf::from(format!("/models/piper/{PIPER_VOICE}.onnx")),
            voice: PIPER_VOICE.to_owned(),
        },
        threads: 3,
        audio_device: None,
    }
}

fn ready_json(
    engine: &str,
    precision: &str,
    voice: &str,
    threads: u32,
    sample_rate: u32,
) -> Vec<u8> {
    format!(
        "{{\"type\":\"ready\",\"protocol\":1,\"pid\":{PID},\"engine\":\"{engine}\",\"precision\":\"{precision}\",\"voice\":\"{voice}\",\"language\":\"en\",\"provider\":\"cpu\",\"threads\":{threads},\"sample_rate\":{sample_rate},\"num_speakers\":1}}\n"
    )
    .into_bytes()
}

fn supertonic_ready_json(sample_rate: u32) -> Vec<u8> {
    ready_json("supertonic-3", "int8", "M5", 2, sample_rate)
}

fn piper_ready_json(sample_rate: u32) -> Vec<u8> {
    ready_json("piper", "fp32", PIPER_VOICE, 3, sample_rate)
}

fn completed_json(sample_rate: u32, num_samples: usize) -> Vec<u8> {
    format!(
        "{{\"type\":\"completed\",\"protocol\":1,\"id\":\"one\",\"sample_rate\":{sample_rate},\"num_samples\":{num_samples},\"wav_bytes\":68}}\n"
    )
    .into_bytes()
}

async fn read_fixture_frame(frame: &[u8]) -> Result<WorkerResponse, WorkerError> {
    let (mut writer, reader) = duplex(MAX_FRAME_BYTES as usize + 1);
    writer.write_all(frame).await.unwrap();
    let mut reader = BufReader::new(reader);
    TtsWorker::read_frame(&mut reader).await
}

async fn validate_ready_frame(config: &TtsConfig, frame: &[u8]) -> Result<SampleRate, WorkerError> {
    let response = read_fixture_frame(frame).await.unwrap();
    TtsWorker::validate_ready(response, &ExpectedReady::for_config(config), PID)
}

async fn validate_completed_frame(
    ready_rate: SampleRate,
    frame: &[u8],
) -> Result<SampleRate, WorkerError> {
    let response = read_fixture_frame(frame).await.unwrap();
    TtsWorker::validate_synthesis_response(response, "one", ready_rate)
}

#[tokio::test]
async fn frame_reading_enforces_cap_json_and_response_correlation() {
    let valid = b"{\"type\":\"completed\",\"protocol\":1,\"id\":\"one\",\"sample_rate\":44100,\"num_samples\":12,\"wav_bytes\":68}\n";
    let response = read_fixture_frame(valid).await.unwrap();
    TtsWorker::validate_synthesis_response(response, "one", rate(44_100)).unwrap();

    let mismatched = read_fixture_frame(valid).await.unwrap();
    assert!(matches!(
        TtsWorker::validate_synthesis_response(mismatched, "two", rate(44_100)),
        Err(WorkerError::MismatchedResponseId)
    ));

    assert!(matches!(
        read_fixture_frame(b"not-json\n").await,
        Err(WorkerError::InvalidJson(_))
    ));

    let mut oversized = vec![b'x'; MAX_FRAME_BYTES as usize];
    oversized.push(b'\n');
    assert!(matches!(
        read_fixture_frame(&oversized).await,
        Err(WorkerError::ResponseTooLarge)
    ));
}

#[tokio::test]
async fn each_engine_accepts_only_its_own_ready_frame() {
    // (config, frame, accepted)
    let cases = [
        (supertonic_config(), supertonic_ready_json(44_100), true),
        (piper_config(), piper_ready_json(22_050), true),
        (supertonic_config(), piper_ready_json(22_050), false),
        (piper_config(), supertonic_ready_json(44_100), false),
        // Right engine, wrong voice, precision or thread count.
        (
            supertonic_config(),
            ready_json("supertonic-3", "int8", "F1", 2, 44_100),
            false,
        ),
        (
            supertonic_config(),
            ready_json("supertonic-3", "fp32", "M5", 2, 44_100),
            false,
        ),
        (
            piper_config(),
            ready_json("piper", "fp32", "en_US-other-medium", 3, 22_050),
            false,
        ),
        (
            piper_config(),
            ready_json("piper", "int8", PIPER_VOICE, 3, 22_050),
            false,
        ),
        (
            piper_config(),
            ready_json("piper", "fp32", PIPER_VOICE, 2, 22_050),
            false,
        ),
    ];

    for (config, frame, accepted) in cases {
        let result = validate_ready_frame(&config, &frame).await;
        let frame = String::from_utf8_lossy(&frame);
        if accepted {
            result.unwrap_or_else(|error| panic!("{frame} should be accepted: {error}"));
        } else {
            assert!(
                matches!(result, Err(WorkerError::InvalidConfiguration)),
                "{frame} should be rejected for {:?}",
                config.engine
            );
        }
    }
}

#[tokio::test]
async fn a_ready_frame_declares_the_rate_the_worker_will_use() {
    assert_eq!(
        validate_ready_frame(&piper_config(), &piper_ready_json(22_050))
            .await
            .unwrap(),
        rate(22_050)
    );
    assert_eq!(
        validate_ready_frame(&supertonic_config(), &supertonic_ready_json(44_100))
            .await
            .unwrap(),
        rate(44_100)
    );
}

#[tokio::test]
async fn the_ready_sample_rate_must_be_within_the_accepted_range() {
    // (rate, accepted): the range edges, one outside each, and the absurd.
    let cases = [
        (0, false),
        (MIN_SAMPLE_RATE_HZ - 1, false),
        (MIN_SAMPLE_RATE_HZ, true),
        (22_050, true),
        (44_100, true),
        (MAX_SAMPLE_RATE_HZ, true),
        (MAX_SAMPLE_RATE_HZ + 1, false),
        (u32::MAX, false),
    ];

    for (hz, accepted) in cases {
        let result = validate_ready_frame(&piper_config(), &piper_ready_json(hz)).await;
        if accepted {
            assert_eq!(result.unwrap(), rate(hz), "{hz} Hz should be accepted");
        } else {
            assert!(
                matches!(result, Err(WorkerError::InvalidConfiguration)),
                "{hz} Hz should be rejected"
            );
        }
    }
}

#[tokio::test]
async fn a_ready_frame_without_a_sample_rate_is_rejected() {
    let frame = b"{\"type\":\"ready\",\"protocol\":1,\"pid\":4242,\"engine\":\"piper\",\"precision\":\"fp32\",\"voice\":\"v\",\"language\":\"en\",\"provider\":\"cpu\",\"threads\":3,\"num_speakers\":1}\n";

    assert!(matches!(
        read_fixture_frame(frame).await,
        Err(WorkerError::InvalidJson(_))
    ));
}

#[tokio::test]
async fn a_chunk_must_use_the_rate_the_worker_declared_when_ready() {
    // (ready rate, chunk rate, accepted)
    let cases = [
        (22_050, 22_050, true),
        (44_100, 44_100, true),
        (22_050, 44_100, false),
        (44_100, 22_050, false),
        // In the accepted range but not what was declared.
        (22_050, 22_051, false),
        // Outside the range, even though the worker declared it.
        (22_050, MIN_SAMPLE_RATE_HZ - 1, false),
        (22_050, MAX_SAMPLE_RATE_HZ + 1, false),
    ];

    for (ready_hz, chunk_hz, accepted) in cases {
        let result = validate_completed_frame(rate(ready_hz), &completed_json(chunk_hz, 12)).await;
        if accepted {
            assert_eq!(result.unwrap(), rate(chunk_hz));
        } else {
            assert!(
                matches!(result, Err(WorkerError::InvalidAudioMetadata)),
                "ready {ready_hz} Hz with a {chunk_hz} Hz chunk should be rejected"
            );
        }
    }
}

#[tokio::test]
async fn a_chunk_may_hold_at_most_120_seconds_of_audio_at_its_rate() {
    for hz in [22_050_u32, 44_100] {
        let limit = usize::try_from(hz * 120).unwrap();
        validate_completed_frame(rate(hz), &completed_json(hz, limit))
            .await
            .unwrap_or_else(|error| panic!("{limit} samples at {hz} Hz should fit: {error}"));
        assert!(
            matches!(
                validate_completed_frame(rate(hz), &completed_json(hz, limit + 1)).await,
                Err(WorkerError::InvalidAudioMetadata)
            ),
            "{} samples at {hz} Hz are too long",
            limit + 1
        );
        assert!(matches!(
            validate_completed_frame(rate(hz), &completed_json(hz, 0)).await,
            Err(WorkerError::InvalidAudioMetadata)
        ));
    }
}

#[test]
fn the_launch_command_follows_the_configured_engine() {
    let supertonic: Vec<_> = worker_arguments(&supertonic_config())
        .into_iter()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        supertonic,
        [
            "-u",
            "/jarvis/scripts/tts/supertonic_worker.py",
            "--model-dir",
            "/models/supertonic",
            "--voice-style",
            "/models/supertonic/voice.bin",
            "--threads",
            "2",
        ]
    );

    let piper: Vec<_> = worker_arguments(&piper_config())
        .into_iter()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        piper,
        [
            "-u",
            "/jarvis/scripts/tts/piper_worker.py",
            "--model",
            "/models/piper/en_GB-northern_english_male-medium.onnx",
            "--threads",
            "3",
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn synthesis_deadline_covers_a_blocked_request_write() {
    let operation = tokio::spawn(async {
        let (mut writer, reader) = duplex(1);
        let mut stdout = BufReader::new(reader);
        TtsWorker::bounded_synthesis(&mut writer, &mut stdout, &[b'x'; 32], "one", rate(44_100))
            .await
    });

    tokio::task::yield_now().await;
    tokio::time::advance(SYNTHESIS_TIMEOUT).await;
    assert!(matches!(
        operation.await.unwrap(),
        Err(WorkerError::SynthesisTimeout)
    ));
}

#[cfg(unix)]
fn spawn_shell_worker(ready: bool, reply: bool) -> TtsWorker {
    spawn_shell_worker_for(
        &supertonic_config(),
        &supertonic_ready_json(44_100),
        44_100,
        ready,
        reply,
    )
}

/// A shell script stands in for the worker: `ready_frame` is its ready line, and it
/// answers every request with a `completed` frame at `completed_rate`.
#[cfg(unix)]
fn spawn_shell_worker_for(
    config: &TtsConfig,
    ready_frame: &[u8],
    completed_rate: u32,
    ready: bool,
    reply: bool,
) -> TtsWorker {
    let ready_frame = String::from_utf8_lossy(ready_frame);
    let ready_json = ready_frame
        .trim_end()
        .replace(&format!("\"pid\":{PID}"), "\"pid\":%s");
    let completed = format!(
        r#"{{"type":"completed","protocol":1,"id":"test-id","sample_rate":{completed_rate},"num_samples":1,"wav_bytes":46}}"#
    );
    let ready_frame = if ready {
        format!(r#"printf '{ready_json}\n' "$$"; "#)
    } else {
        String::new()
    };
    let request_loop = if reply {
        format!(r#"while IFS= read -r line; do printf '{completed}\n'; done"#)
    } else if ready {
        format!(
            r#"IFS= read -r line || exit; IFS= read -r release || exit; printf '{completed}\n'; while IFS= read -r line; do :; done"#
        )
    } else {
        "while IFS= read -r line; do :; done".to_owned()
    };
    let mut child = Command::new("/bin/sh")
        .arg("-c")
        .arg(format!("{ready_frame}{request_loop}"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();

    TtsWorker {
        child: Some(child),
        stdin: Some(stdin),
        stdout: BufReader::new(stdout),
        state: WorkerState::Starting,
        expected: ExpectedReady::for_config(config),
        sample_rate: None,
    }
}

#[cfg(unix)]
#[tokio::test]
async fn successful_responses_keep_worker_idle_for_reuse_and_eof_shutdown_reaps() {
    let mut worker = spawn_shell_worker(true, true);
    worker.wait_ready().await.unwrap();

    for _ in 0..2 {
        let sample_rate = worker
            .speak(
                "test-id".to_owned(),
                "hello".to_owned(),
                "/tmp/answer.wav".to_owned(),
            )
            .await
            .unwrap();
        assert_eq!(sample_rate, rate(44_100));
        assert!(worker.is_idle());
    }

    worker.shutdown_idle().await.unwrap();
    assert_eq!(worker.state, WorkerState::Terminated);
    assert!(worker.child.is_none());
}

#[cfg(unix)]
#[tokio::test]
async fn a_piper_worker_becomes_ready_and_speaks_at_its_declared_rate() {
    let mut worker = spawn_shell_worker_for(
        &piper_config(),
        &piper_ready_json(22_050),
        22_050,
        true,
        true,
    );
    worker.wait_ready().await.unwrap();

    let sample_rate = worker
        .speak(
            "test-id".to_owned(),
            "hello".to_owned(),
            "/tmp/answer.wav".to_owned(),
        )
        .await
        .unwrap();

    assert_eq!(sample_rate, rate(22_050));
    assert!(worker.is_idle());
    worker.terminate().await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn a_worker_that_is_the_wrong_engine_never_becomes_ready() {
    // Supertonic is configured but a Piper worker answers.
    let mut worker = spawn_shell_worker_for(
        &supertonic_config(),
        &piper_ready_json(22_050),
        22_050,
        true,
        true,
    );

    assert!(matches!(
        worker.wait_ready().await,
        Err(WorkerError::InvalidConfiguration)
    ));
    assert!(!worker.is_idle());
    worker.terminate().await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn cancelled_startup_and_synthesis_require_kill_and_reap_before_reuse() {
    let mut starting = spawn_shell_worker(false, false);
    let mut startup = Box::pin(starting.wait_ready());
    tokio::select! {
        biased;
        result = &mut startup => panic!("startup unexpectedly completed: {result:?}"),
        _ = std::future::ready(()) => {},
    }
    drop(startup);
    assert_eq!(starting.state, WorkerState::WaitingReady);
    assert!(matches!(
        starting.wait_ready().await,
        Err(WorkerError::InvalidState)
    ));
    starting.terminate().await.unwrap();
    assert!(starting.child.is_none());

    let mut synthesizing = spawn_shell_worker(true, false);
    synthesizing.wait_ready().await.unwrap();
    let mut synthesis = Box::pin(synthesizing.speak(
        "test-id".to_owned(),
        "hello".to_owned(),
        "/tmp/answer.wav".to_owned(),
    ));
    tokio::select! {
        biased;
        result = &mut synthesis => panic!("synthesis unexpectedly completed: {result:?}"),
        _ = std::future::ready(()) => {},
    }
    drop(synthesis);
    synthesizing
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"release\n")
        .await
        .unwrap();
    let late_response = TtsWorker::read_frame(&mut synthesizing.stdout)
        .await
        .unwrap();
    TtsWorker::validate_synthesis_response(late_response, "test-id", rate(44_100)).unwrap();
    assert_eq!(synthesizing.state, WorkerState::Synthesizing);
    assert!(!synthesizing.is_idle());
    assert!(matches!(
        synthesizing
            .speak(
                "test-id".to_owned(),
                "hello".to_owned(),
                "/tmp/answer.wav".to_owned(),
            )
            .await,
        Err(WorkerError::InvalidState)
    ));
    synthesizing.terminate().await.unwrap();
    assert!(synthesizing.child.is_none());
}
