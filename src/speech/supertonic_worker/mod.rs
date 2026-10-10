mod error;
mod protocol;

use std::{path::Path, process::Stdio, time::Duration};

use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
    time::{Instant, timeout},
};

pub(crate) use error::WorkerError;

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

pub(super) struct SupertonicWorker {
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
#[path = "../../../tests/unit/speech/supertonic_worker_tests.rs"]
mod tests;
