mod error;
mod protocol;

use std::{ffi::OsString, process::Stdio, time::Duration};

use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
    time::{Instant, timeout},
};

pub(crate) use error::WorkerError;

use protocol::{PROTOCOL_VERSION, WorkerRequest, WorkerResponse};

use super::sample_rate::SampleRate;
use crate::config::{TtsConfig, TtsEngine};

const MAX_FRAME_BYTES: u64 = 4 * 1024;
const MAX_REQUEST_BYTES: usize = 32 * 1024;
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

/// Supertonic bundle values the worker must report; Piper's come from its voice setting.
const SUPERTONIC_ENGINE: &str = "supertonic-3";
const SUPERTONIC_PRECISION: &str = "int8";
const SUPERTONIC_VOICE: &str = "M5";
const SUPERTONIC_VOICE_STYLE_FILE: &str = "voice.bin";
const PIPER_ENGINE: &str = "piper";
const PIPER_PRECISION: &str = "fp32";

/// What a worker's ready frame must say, taken from the configured engine.
///
/// Both engines are single-speaker, English, CPU workers. The sample rate is not
/// pinned here: each voice declares its own, which must fall in the accepted range.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ExpectedReady {
    engine: &'static str,
    precision: &'static str,
    voice: String,
    threads: u32,
}

impl ExpectedReady {
    fn for_config(config: &TtsConfig) -> Self {
        let (engine, precision, voice) = match &config.engine {
            TtsEngine::Supertonic { .. } => (
                SUPERTONIC_ENGINE,
                SUPERTONIC_PRECISION,
                SUPERTONIC_VOICE.to_owned(),
            ),
            TtsEngine::Piper { voice, .. } => (PIPER_ENGINE, PIPER_PRECISION, voice.clone()),
        };
        Self {
            engine,
            precision,
            voice,
            threads: config.threads,
        }
    }
}

/// Arguments after the interpreter: `-u`, the worker script, the engine's model arguments
/// and the thread count.
fn worker_arguments(config: &TtsConfig) -> Vec<OsString> {
    let mut arguments = vec![OsString::from("-u"), config.worker_script.clone().into()];
    match &config.engine {
        TtsEngine::Supertonic { model_dir } => {
            arguments.extend([
                OsString::from("--model-dir"),
                model_dir.clone().into(),
                OsString::from("--voice-style"),
                model_dir.join(SUPERTONIC_VOICE_STYLE_FILE).into(),
            ]);
        }
        TtsEngine::Piper { model, .. } => {
            arguments.extend([OsString::from("--model"), model.clone().into()]);
        }
    }
    arguments.extend([
        OsString::from("--threads"),
        config.threads.to_string().into(),
    ]);
    arguments
}

/// One running TTS worker process, Supertonic or Piper, speaking protocol v1.
pub(super) struct TtsWorker {
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    state: WorkerState,
    expected: ExpectedReady,
    /// The rate the ready frame declared; every synthesized chunk must match it.
    /// Set once the worker is ready.
    sample_rate: Option<SampleRate>,
}

impl TtsWorker {
    fn spawn_process(config: &TtsConfig) -> Result<Child, WorkerError> {
        Command::new(&config.python)
            .args(worker_arguments(config))
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
    pub fn launch(config: &TtsConfig) -> Result<Self, WorkerError> {
        let mut child = Self::spawn_process(config)?;
        let stdin = child.stdin.take().ok_or(WorkerError::MissingStdin)?;
        let stdout = child.stdout.take().ok_or(WorkerError::MissingStdout)?;

        Ok(Self {
            child: Some(child),
            stdin: Some(stdin),
            stdout: BufReader::new(stdout),
            state: WorkerState::Starting,
            expected: ExpectedReady::for_config(config),
            sample_rate: None,
        })
    }

    pub fn is_idle(&self) -> bool {
        self.state == WorkerState::Idle && self.child.is_some() && self.stdin.is_some()
    }

    pub async fn wait_ready(&mut self) -> Result<(), WorkerError> {
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

        let sample_rate = Self::validate_ready(response, &self.expected, expected_pid)?;

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

        self.sample_rate = Some(sample_rate);
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

    /// Returns the sample rate the worker declared.
    fn validate_ready(
        response: WorkerResponse,
        expected: &ExpectedReady,
        expected_pid: u32,
    ) -> Result<SampleRate, WorkerError> {
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
            || engine != expected.engine
            || precision != expected.precision
            || voice != expected.voice
            || language != "en"
            || provider != "cpu"
            || ready_threads != expected.threads
            || num_speakers != 1
        {
            return Err(WorkerError::InvalidConfiguration);
        }

        SampleRate::new(sample_rate).ok_or(WorkerError::InvalidConfiguration)
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
        ready_rate: SampleRate,
    ) -> Result<SampleRate, WorkerError>
    where
        W: AsyncWrite + Unpin,
        R: AsyncBufRead + Unpin,
    {
        let transaction = async {
            Self::write_request(stdin, request).await?;
            let response = Self::read_frame(stdout).await?;
            Self::validate_synthesis_response(response, expected_id, ready_rate)
        };

        match timeout(SYNTHESIS_TIMEOUT, transaction).await {
            Ok(result) => result,
            Err(_) => Err(WorkerError::SynthesisTimeout),
        }
    }

    /// Returns the sample rate of the chunk, which must be the rate the worker declared
    /// when it became ready.
    fn validate_synthesis_response(
        response: WorkerResponse,
        expected_id: &str,
        ready_rate: SampleRate,
    ) -> Result<SampleRate, WorkerError> {
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

                let sample_rate = SampleRate::new(sample_rate)
                    .filter(|rate| *rate == ready_rate)
                    .ok_or(WorkerError::InvalidAudioMetadata)?;
                if num_samples == 0
                    || num_samples > sample_rate.max_samples()
                    || wav_bytes == 0
                    || wav_bytes > MAX_WAV_BYTES
                {
                    return Err(WorkerError::InvalidAudioMetadata);
                }

                Ok(sample_rate)
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
    ) -> Result<SampleRate, WorkerError> {
        if !self.is_idle() {
            return Err(WorkerError::InvalidState);
        }
        let ready_rate = self.sample_rate.ok_or(WorkerError::InvalidState)?;

        let request = Self::serialize_speak_request(id.clone(), text, output_path)?;
        self.state = WorkerState::Synthesizing;

        let stdin = self.stdin.as_mut().ok_or(WorkerError::MissingStdin)?;
        let stdout = &mut self.stdout;
        match Self::bounded_synthesis(stdin, stdout, &request, &id, ready_rate).await {
            Ok(sample_rate) => {
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
                Ok(sample_rate)
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
#[path = "../../../../tests/unit/features/speech/tts_worker/tts_worker_tests.rs"]
mod tests;
