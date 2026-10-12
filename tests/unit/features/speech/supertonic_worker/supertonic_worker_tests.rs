use super::*;
use tokio::io::{AsyncWriteExt, duplex};

async fn read_fixture_frame(frame: &[u8]) -> Result<WorkerResponse, WorkerError> {
    let (mut writer, reader) = duplex(MAX_FRAME_BYTES as usize + 1);
    writer.write_all(frame).await.unwrap();
    let mut reader = BufReader::new(reader);
    SupertonicWorker::read_frame(&mut reader).await
}

#[tokio::test]
async fn frame_reading_enforces_cap_json_and_response_correlation() {
    let valid = b"{\"type\":\"completed\",\"protocol\":1,\"id\":\"one\",\"sample_rate\":44100,\"num_samples\":12,\"wav_bytes\":68}\n";
    let response = read_fixture_frame(valid).await.unwrap();
    SupertonicWorker::validate_synthesis_response(response, "one").unwrap();

    let mismatched = read_fixture_frame(valid).await.unwrap();
    assert!(matches!(
        SupertonicWorker::validate_synthesis_response(mismatched, "two"),
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

#[tokio::test(start_paused = true)]
async fn synthesis_deadline_covers_a_blocked_request_write() {
    let operation = tokio::spawn(async {
        let (mut writer, reader) = duplex(1);
        let mut stdout = BufReader::new(reader);
        SupertonicWorker::bounded_synthesis(&mut writer, &mut stdout, &[b'x'; 32], "one").await
    });

    tokio::task::yield_now().await;
    tokio::time::advance(SYNTHESIS_TIMEOUT).await;
    assert!(matches!(
        operation.await.unwrap(),
        Err(WorkerError::SynthesisTimeout)
    ));
}

#[cfg(unix)]
fn spawn_shell_worker(ready: bool, reply: bool) -> SupertonicWorker {
    let ready_frame = if ready {
        r#"printf '{"type":"ready","protocol":1,"pid":%s,"engine":"supertonic-3","precision":"int8","voice":"M5","language":"en","provider":"cpu","threads":2,"sample_rate":44100,"num_speakers":1}\n' "$$"; "#
    } else {
        ""
    };
    let request_loop = if reply {
        r#"while IFS= read -r line; do printf '{"type":"completed","protocol":1,"id":"test-id","sample_rate":44100,"num_samples":1,"wav_bytes":46}\n'; done"#
    } else if ready {
        r#"IFS= read -r line || exit; IFS= read -r release || exit; printf '{"type":"completed","protocol":1,"id":"test-id","sample_rate":44100,"num_samples":1,"wav_bytes":46}\n'; while IFS= read -r line; do :; done"#
    } else {
        "while IFS= read -r line; do :; done"
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

    SupertonicWorker {
        child: Some(child),
        stdin: Some(stdin),
        stdout: BufReader::new(stdout),
        state: WorkerState::Starting,
    }
}

#[cfg(unix)]
#[tokio::test]
async fn successful_responses_keep_worker_idle_for_reuse_and_eof_shutdown_reaps() {
    let mut worker = spawn_shell_worker(true, true);
    worker.wait_ready(2).await.unwrap();

    for _ in 0..2 {
        worker
            .speak(
                "test-id".to_owned(),
                "hello".to_owned(),
                "/tmp/answer.wav".to_owned(),
            )
            .await
            .unwrap();
        assert!(worker.is_idle());
    }

    worker.shutdown_idle().await.unwrap();
    assert_eq!(worker.state, WorkerState::Terminated);
    assert!(worker.child.is_none());
}

#[cfg(unix)]
#[tokio::test]
async fn cancelled_startup_and_synthesis_require_kill_and_reap_before_reuse() {
    let mut starting = spawn_shell_worker(false, false);
    let mut startup = Box::pin(starting.wait_ready(2));
    tokio::select! {
        biased;
        result = &mut startup => panic!("startup unexpectedly completed: {result:?}"),
        _ = std::future::ready(()) => {},
    }
    drop(startup);
    assert_eq!(starting.state, WorkerState::WaitingReady);
    assert!(matches!(
        starting.wait_ready(2).await,
        Err(WorkerError::InvalidState)
    ));
    starting.terminate().await.unwrap();
    assert!(starting.child.is_none());

    let mut synthesizing = spawn_shell_worker(true, false);
    synthesizing.wait_ready(2).await.unwrap();
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
    let late_response = SupertonicWorker::read_frame(&mut synthesizing.stdout)
        .await
        .unwrap();
    SupertonicWorker::validate_synthesis_response(late_response, "test-id").unwrap();
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
