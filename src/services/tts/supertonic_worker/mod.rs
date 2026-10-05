mod error;
mod protocol;

use std::{path::Path, process::Stdio, time::Duration};

use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
    time::timeout,
};

pub use error::WorkerError;

use protocol::{PROTOCOL_VERSION, WorkerRequest, WorkerResponse};

const MAX_FRAME_BYTES: u64 = 4 * 1024;
const MAX_REQUEST_BYTES: usize = 32 * 1024;
const STARTUP_TIMEOUT: Duration = Duration::from_secs(60);
const SYNTHESIS_TIMEOUT: Duration = Duration::from_secs(30);

pub struct SupertonicWorker {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
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

    async fn take_stdin(child: &mut Child) -> Result<ChildStdin, WorkerError> {
        match child.stdin.take() {
            Some(stdin) => Ok(stdin),
            None => {
                Self::terminate_worker(child).await?;
                Err(WorkerError::MissingStdin)
            }
        }
    }

    async fn take_stdout(child: &mut Child) -> Result<ChildStdout, WorkerError> {
        match child.stdout.take() {
            Some(stdout) => Ok(stdout),
            None => {
                Self::terminate_worker(child).await?;
                Err(WorkerError::MissingStdout)
            }
        }
    }
    async fn read_synthesis_frame(
        stdout: &mut BufReader<ChildStdout>,
    ) -> Result<WorkerResponse, WorkerError> {
        let mut bytes = Vec::new();

        let read_result = timeout(
            SYNTHESIS_TIMEOUT,
            stdout
                .take(MAX_FRAME_BYTES + 1)
                .read_until(b'\n', &mut bytes),
        )
        .await;

        let bytes_read = match read_result {
            Ok(Ok(bytes_read)) => bytes_read,
            Ok(Err(_)) => return Err(WorkerError::WorkerEof),
            Err(_) => return Err(WorkerError::SynthesisTimeout),
        };

        if bytes_read == 0 {
            return Err(WorkerError::WorkerEof);
        }

        if bytes.len() > MAX_FRAME_BYTES as usize || !bytes.ends_with(b"\n") {
            return Err(WorkerError::ResponseTooLarge);
        }

        serde_json::from_slice(&bytes).map_err(WorkerError::InvalidJson)
    }
    async fn read_startup_frame(
        stdout: &mut BufReader<ChildStdout>,
    ) -> Result<WorkerResponse, WorkerError> {
        let mut bytes = Vec::new();

        let read_result = timeout(
            STARTUP_TIMEOUT,
            stdout
                .take(MAX_FRAME_BYTES + 1)
                .read_until(b'\n', &mut bytes),
        )
        .await;

        let bytes_read = match read_result {
            Ok(Ok(bytes_read)) => bytes_read,
            Ok(Err(_)) => return Err(WorkerError::StartupEof),
            Err(_) => return Err(WorkerError::StartupTimeout),
        };

        if bytes_read == 0 {
            return Err(WorkerError::StartupEof);
        }

        if bytes.len() > MAX_FRAME_BYTES as usize || !bytes.ends_with(b"\n") {
            return Err(WorkerError::ResponseTooLarge);
        }

        serde_json::from_slice(&bytes).map_err(WorkerError::InvalidJson)
    }

    fn validate_ready(response: WorkerResponse, threads: u32) -> Result<(), WorkerError> {
        let WorkerResponse::Ready {
            protocol,
            engine,
            precision,
            voice,
            language,
            provider,
            threads: ready_threads,
            sample_rate,
            num_speakers,
            ..
        } = response
        else {
            return Err(WorkerError::UnexpectedStartupFrame);
        };

        if protocol != PROTOCOL_VERSION {
            return Err(WorkerError::UnsupportedProtocol(protocol));
        }

        if engine != "supertonic-3"
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

    async fn write_request(stdin: &mut ChildStdin, request: &[u8]) -> Result<(), WorkerError> {
        stdin.write_all(request).await.map_err(WorkerError::Write)?;

        stdin.flush().await.map_err(WorkerError::Write)?;

        Ok(())
    }

    async fn terminate_worker(child: &mut Child) -> Result<(), WorkerError> {
        match child.try_wait() {
            Ok(Some(_)) => return Ok(()),
            Ok(None) => {}
            Err(_) => return Err(WorkerError::CleanupFailed),
        }

        child.start_kill().map_err(|_| WorkerError::CleanupFailed)?;

        child.wait().await.map_err(|_| WorkerError::CleanupFailed)?;

        Ok(())
    }
    fn validate_synthesis_response(
        response: WorkerResponse,
        expected_id: &str,
    ) -> Result<(), WorkerError> {
        match response {
            WorkerResponse::Completed { protocol, id, .. } => {
                if protocol != PROTOCOL_VERSION {
                    return Err(WorkerError::UnsupportedProtocol(protocol));
                }

                if id != expected_id {
                    return Err(WorkerError::MismatchedResponseId);
                }

                Ok(())
            }

            WorkerResponse::Error {
                protocol,
                id,
                error,
            } => {
                if protocol != PROTOCOL_VERSION {
                    return Err(WorkerError::UnsupportedProtocol(protocol));
                }

                if id.as_deref() != Some(expected_id) {
                    return Err(WorkerError::MismatchedResponseId);
                }

                Err(WorkerError::WorkerReported(error))
            }

            WorkerResponse::Ready { .. } => Err(WorkerError::UnexpectedResponse),
        }
    }

    pub async fn spawn(
        python: &Path,
        worker_script: &Path,
        model_dir: &Path,
        voice_style: &Path,
        threads: u32,
    ) -> Result<Self, WorkerError> {
        let mut child =
            Self::spawn_process(python, worker_script, model_dir, voice_style, threads)?;

        let stdin = Self::take_stdin(&mut child).await?;
        let stdout = Self::take_stdout(&mut child).await?;
        let mut stdout = BufReader::new(stdout);

        let response = match Self::read_startup_frame(&mut stdout).await {
            Ok(response) => response,
            Err(error) => {
                Self::terminate_worker(&mut child).await?;
                return Err(error);
            }
        };

        if let Err(error) = Self::validate_ready(response, threads) {
            Self::terminate_worker(&mut child).await?;
            return Err(error);
        }

        Ok(Self {
            child,
            stdin,
            stdout,
        })
    }
    async fn remove_partial_output(output_path: &str) {
        match tokio::fs::remove_file(output_path).await {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                eprintln!("warning: failed to remove partial TTS output {output_path}: {error}");
            }
        }
    }
    pub async fn speak(
        &mut self,
        id: String,
        text: String,
        output_path: String,
    ) -> Result<(), WorkerError> {
        let request = Self::serialize_speak_request(id.clone(), text, output_path.clone())?;

        if let Err(error) = Self::write_request(&mut self.stdin, &request).await {
            Self::remove_partial_output(&output_path).await;
            Self::terminate_worker(&mut self.child).await?;
            return Err(error);
        }

        let response = match Self::read_synthesis_frame(&mut self.stdout).await {
            Ok(response) => response,
            Err(error) => {
                Self::remove_partial_output(&output_path).await;
                Self::terminate_worker(&mut self.child).await?;
                return Err(error);
            }
        };

        match Self::validate_synthesis_response(response, &id) {
            Ok(()) => Ok(()),

            Err(error @ WorkerError::WorkerReported(_)) => {
                Self::remove_partial_output(&output_path).await;
                Err(error)
            }

            Err(error) => {
                Self::remove_partial_output(&output_path).await;
                Self::terminate_worker(&mut self.child).await?;
                Err(error)
            }
        }
    }
}
