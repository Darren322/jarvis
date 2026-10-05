mod error;
mod protocol;

use std::{path::Path, process::Stdio, time::Duration};

use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
    time::{Instant, timeout},
};

pub use error::WorkerError;

use protocol::{PROTOCOL_VERSION, WorkerRequest, WorkerResponse};

const MAX_FRAME_BYTES: u64 = 4 * 1024;
const MAX_REQUEST_BYTES: usize = 32 * 1024;
const MAX_AUDIO_SAMPLES: usize = 5_292_000;
const MAX_WAV_BYTES: usize = 16 * 1024 * 1024;
const STARTUP_TIMEOUT: Duration = Duration::from_secs(60);
const SYNTHESIS_TIMEOUT: Duration = Duration::from_secs(30);
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(2);
const IDLE_SHUTDOWN_GRACE: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WorkerState {
    Starting,
    WaitingReady,
    Idle,
    Synthesizing,
    Closing,
    Reaping,
    Terminated,
}

pub struct SupertonicWorker {
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    state: WorkerState,
}

impl SupertonicWorker {
    fn spawn_process(
        python: &Path,
        worker_script: &Path,
        model_dir: &Path,
        voice_style: &Path,
        threads: u32,
    ) -> Result<Child, WorkerError> {
        Command::new(python)
            .arg("-u")
            .arg(worker_script)
            .arg("--model-dir")
            .arg(model_dir)
            .arg("--voice-style")
            .arg(voice_style)
            .arg("--threads")
            .arg(threads.to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(WorkerError::Spawn)
    }

    /// Starts the worker and returns its owned process before waiting for readiness.
    ///
    /// If the caller cancels `wait_ready`, it must call `terminate` before dropping
    /// this owner or attempting another operation.
    pub fn launch(
        python: &Path,
        worker_script: &Path,
        model_dir: &Path,
        voice_style: &Path,
        threads: u32,
    ) -> Result<Self, WorkerError> {
        let mut child =
            Self::spawn_process(python, worker_script, model_dir, voice_style, threads)?;
        let stdin = child.stdin.take().ok_or(WorkerError::MissingStdin)?;
        let stdout = child.stdout.take().ok_or(WorkerError::MissingStdout)?;

        Ok(Self {
            child: Some(child),
            stdin: Some(stdin),
            stdout: BufReader::new(stdout),
            state: WorkerState::Starting,
        })
    }

    pub fn is_idle(&self) -> bool {
        self.state == WorkerState::Idle && self.child.is_some() && self.stdin.is_some()
    }

    pub async fn wait_ready(&mut self, threads: u32) -> Result<(), WorkerError> {
        if self.state != WorkerState::Starting {
            return Err(WorkerError::InvalidState);
        }

        // The ready-frame future may be dropped after consuming only part of a
        // frame. Keep the worker unusable until this attempt completes.
        self.state = WorkerState::WaitingReady;

        let expected_pid = self
            .child
            .as_ref()
            .and_then(Child::id)
            .ok_or(WorkerError::WorkerExited)?;

        let read_result = timeout(STARTUP_TIMEOUT, Self::read_frame(&mut self.stdout)).await;
        let response = match read_result {
            Ok(Ok(response)) => response,
            Ok(Err(WorkerError::WorkerEof)) => return Err(WorkerError::StartupEof),
            Ok(Err(error)) => return Err(error),
            Err(_) => return Err(WorkerError::StartupTimeout),
        };

        Self::validate_ready(response, threads, expected_pid)?;

        if self
            .child
            .as_mut()
            .ok_or(WorkerError::WorkerExited)?
            .try_wait()
            .map_err(WorkerError::ChildStatus)?
            .is_some()
        {
            return Err(WorkerError::WorkerExited);
        }

        self.state = WorkerState::Idle;
        Ok(())
    }

    async fn read_frame<R>(stdout: &mut R) -> Result<WorkerResponse, WorkerError>
    where
        R: AsyncBufRead + Unpin,
    {
        let mut bytes = Vec::new();
        let bytes_read = stdout
            .take(MAX_FRAME_BYTES + 1)
            .read_until(b'\n', &mut bytes)
            .await
            .map_err(WorkerError::Read)?;

        if bytes_read == 0 {
            return Err(WorkerError::WorkerEof);
        }

        if bytes.len() > MAX_FRAME_BYTES as usize || !bytes.ends_with(b"\n") {
            return Err(WorkerError::ResponseTooLarge);
        }

        serde_json::from_slice(&bytes).map_err(WorkerError::InvalidJson)
    }

    fn validate_ready(
        response: WorkerResponse,
        threads: u32,
        expected_pid: u32,
    ) -> Result<(), WorkerError> {
        let WorkerResponse::Ready {
            protocol,
            pid,
            engine,
            precision,
            voice,
            language,
            provider,
            threads: ready_threads,
            sample_rate,
            num_speakers,
        } = response
        else {
            return Err(WorkerError::UnexpectedStartupFrame);
        };

        if protocol != PROTOCOL_VERSION {
            return Err(WorkerError::UnsupportedProtocol(protocol));
        }

        if pid != expected_pid
            || engine != "supertonic-3"
            || precision != "int8"
            || voice != "M5"
            || language != "en"
            || provider != "cpu"
            || ready_threads != threads
            || sample_rate != 44_100
            || num_speakers != 1
        {
            return Err(WorkerError::InvalidConfiguration);
        }

        Ok(())
    }

    fn serialize_speak_request(
        id: String,
        text: String,
        output_path: String,
    ) -> Result<Vec<u8>, WorkerError> {
        let request = WorkerRequest::Speak {
            protocol: PROTOCOL_VERSION,
            id,
            text,
            output_path,
        };

        let mut bytes = serde_json::to_vec(&request).map_err(WorkerError::InvalidJson)?;
        bytes.push(b'\n');

        if bytes.len() > MAX_REQUEST_BYTES {
            return Err(WorkerError::RequestTooLarge);
        }

        Ok(bytes)
    }

    async fn write_request<W>(stdin: &mut W, request: &[u8]) -> Result<(), WorkerError>
    where
        W: AsyncWrite + Unpin,
    {
        stdin.write_all(request).await.map_err(WorkerError::Write)?;
        stdin.flush().await.map_err(WorkerError::Write)?;
        Ok(())
    }

    async fn bounded_synthesis<W, R>(
        stdin: &mut W,
        stdout: &mut R,
        request: &[u8],
        expected_id: &str,
    ) -> Result<(), WorkerError>
    where
        W: AsyncWrite + Unpin,
        R: AsyncBufRead + Unpin,
    {
        let transaction = async {
            Self::write_request(stdin, request).await?;
            let response = Self::read_frame(stdout).await?;
            Self::validate_synthesis_response(response, expected_id)
        };

        match timeout(SYNTHESIS_TIMEOUT, transaction).await {
            Ok(result) => result,
            Err(_) => Err(WorkerError::SynthesisTimeout),
        }
    }

    fn validate_synthesis_response(
        response: WorkerResponse,
        expected_id: &str,
    ) -> Result<(), WorkerError> {
        match response {
            WorkerResponse::Completed {
                protocol,
                id,
                sample_rate,
                num_samples,
                wav_bytes,
            } => {
                if protocol != PROTOCOL_VERSION {
                    return Err(WorkerError::UnsupportedProtocol(protocol));
                }

                if id != expected_id {
                    return Err(WorkerError::MismatchedResponseId);
                }

                if sample_rate != 44_100
                    || num_samples == 0
                    || num_samples > MAX_AUDIO_SAMPLES
                    || wav_bytes == 0
                    || wav_bytes > MAX_WAV_BYTES
                {
                    return Err(WorkerError::InvalidAudioMetadata);
                }

                Ok(())
            }

            WorkerResponse::Error { protocol, id, .. } => {
                if protocol != PROTOCOL_VERSION {
                    return Err(WorkerError::UnsupportedProtocol(protocol));
                }

                if id.as_deref() != Some(expected_id) {
                    return Err(WorkerError::MismatchedResponseId);
                }

                Err(WorkerError::WorkerReported)
            }

            WorkerResponse::Ready { .. } => Err(WorkerError::UnexpectedResponse),
        }
    }

    pub async fn speak(
        &mut self,
        id: String,
        text: String,
        output_path: String,
    ) -> Result<(), WorkerError> {
        if !self.is_idle() {
            return Err(WorkerError::InvalidState);
        }

        let request = Self::serialize_speak_request(id.clone(), text, output_path)?;
        self.state = WorkerState::Synthesizing;

        let stdin = self.stdin.as_mut().ok_or(WorkerError::MissingStdin)?;
        let stdout = &mut self.stdout;
        match Self::bounded_synthesis(stdin, stdout, &request, &id).await {
            Ok(()) => {
                if self
                    .child
                    .as_mut()
                    .ok_or(WorkerError::WorkerExited)?
                    .try_wait()
                    .map_err(WorkerError::ChildStatus)?
                    .is_some()
                {
                    return Err(WorkerError::WorkerExited);
                }

                self.state = WorkerState::Idle;
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    fn mark_reaped(&mut self) {
        self.child.take();
        self.stdin.take();
        self.state = WorkerState::Terminated;
    }

    fn observe_exit(&mut self) -> Result<bool, WorkerError> {
        let status = self
            .child
            .as_mut()
            .ok_or(WorkerError::WorkerExited)?
            .try_wait()
            .map_err(|_| WorkerError::CleanupFailed)?;

        if status.is_some() {
            self.mark_reaped();
            Ok(true)
        } else {
            Ok(false)
        }
    }

    async fn wait_until(&mut self, deadline: Instant) -> Result<(), WorkerError> {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(WorkerError::CleanupTimedOut);
        }

        let wait_result = timeout(
            remaining,
            self.child.as_mut().ok_or(WorkerError::WorkerExited)?.wait(),
        )
        .await;

        match wait_result {
            Ok(Ok(_)) => {
                self.mark_reaped();
                Ok(())
            }
            Ok(Err(_)) => Err(WorkerError::CleanupFailed),
            Err(_) => Err(WorkerError::CleanupTimedOut),
        }
    }

    async fn kill_and_reap_until(&mut self, deadline: Instant) -> Result<(), WorkerError> {
        if self.child.is_none() {
            self.state = WorkerState::Terminated;
            return Ok(());
        }

        if self.observe_exit()? {
            return Ok(());
        }

        if self.state != WorkerState::Reaping {
            let kill_result = self
                .child
                .as_mut()
                .ok_or(WorkerError::WorkerExited)?
                .start_kill();

            if kill_result.is_err() {
                if self.observe_exit()? {
                    return Ok(());
                }

                // The process may have exited between try_wait and start_kill.
                // Wait only to the cleanup deadline; keep the pre-reap state so
                // a later cleanup attempt can retry the kill if it is still live.
                return self.wait_until(deadline).await;
            }

            self.state = WorkerState::Reaping;
        }

        self.wait_until(deadline).await
    }

    /// Kills the worker and waits for its process to be reaped.
    ///
    /// On timeout or another cleanup error the child remains owned by `self`, so
    /// the caller can retry cleanup and must not discard the owner.
    pub async fn terminate(&mut self) -> Result<(), WorkerError> {
        self.stdin.take();
        self.kill_and_reap_until(Instant::now() + CLEANUP_TIMEOUT)
            .await
    }

    /// Closes stdin and gives an idle worker a short chance to exit normally,
    /// then kills and reaps it before the shared cleanup deadline.
    pub async fn shutdown_idle(&mut self) -> Result<(), WorkerError> {
        if self.child.is_none() {
            self.state = WorkerState::Terminated;
            return Ok(());
        }

        if self.state == WorkerState::Closing || self.state == WorkerState::Reaping {
            return self.terminate().await;
        }

        if !self.is_idle() {
            return Err(WorkerError::InvalidState);
        }

        let deadline = Instant::now() + CLEANUP_TIMEOUT;
        let graceful_deadline = (Instant::now() + IDLE_SHUTDOWN_GRACE).min(deadline);
        self.state = WorkerState::Closing;
        self.stdin.take();

        match self.wait_until(graceful_deadline).await {
            Ok(()) => Ok(()),
            Err(WorkerError::CleanupTimedOut | WorkerError::CleanupFailed) => {
                self.kill_and_reap_until(deadline).await
            }
            Err(error) => Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
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
}
