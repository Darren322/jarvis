use std::{
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::Duration,
};

use tempfile::{Builder, TempDir};
use tokio::{task::JoinHandle, time::timeout};

use crate::config::TtsConfig;

use super::{
    playback::{AudioPlayer, PlaybackError},
    supertonic_worker::{SupertonicWorker, WorkerError},
};

const MAX_SPEECH_TEXT_BYTES: usize = 4 * 1024;
const EXPECTED_SAMPLE_RATE: u32 = 44_100;
const EXPECTED_CHANNELS: u16 = 1;
const EXPECTED_BITS_PER_SAMPLE: u16 = 16;
const MAX_WAV_SAMPLES: usize = 5_292_000;
const MAX_WAV_BYTES: u64 = 16 * 1024 * 1024;
const PLAYBACK_GRACE: Duration = Duration::from_secs(2);
const MAX_PLAYBACK_DURATION: Duration = Duration::from_secs(122);
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(2);
const PLAYER_INITIALIZATION_TIMEOUT: Duration = Duration::from_secs(60);
const WAV_DECODE_TIMEOUT: Duration = Duration::from_secs(30);
const PLAYBACK_POLL_INTERVAL: Duration = Duration::from_millis(20);

struct ValidatedWav {
    samples: Vec<f32>,
    duration: Duration,
}

impl ValidatedWav {
    fn playback_deadline(&self) -> Duration {
        self.duration
            .saturating_add(PLAYBACK_GRACE)
            .min(MAX_PLAYBACK_DURATION)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SpeechStage {
    Starting,
    Synthesizing,
    Decoding,
    Playing,
}

struct ActiveSpeech {
    stage: SpeechStage,
    operation_dir: Option<TempDir>,
    decode_task: Option<JoinHandle<Result<ValidatedWav, SpeechError>>>,
}

#[derive(Debug)]
pub enum CleanupFailure {
    Worker(WorkerError),
    TemporaryDirectory(std::io::Error),
    BlockingTaskTimeout,
}

#[derive(Debug)]
pub enum SpeechError {
    TextTooLarge,
    Disabled,
    AlreadyActive,
    CreateTemp(std::io::Error),
    InvalidTempPath,
    WorkerLaunch(WorkerError),
    WorkerStartup(WorkerError),
    Synthesis(WorkerError),
    Playback(PlaybackError),
    PlaybackDeviceFailed,
    PlaybackTimedOut,
    PlaybackInitializationTimedOut,
    PlaybackInitializationTaskFailed,
    DecodeTimedOut,
    DecodeTaskFailed,
    OpenWav(hound::Error),
    DecodeWav(hound::Error),
    InvalidWavFormat,
    EmptyWav,
    WavTooLong,
    WavTooLarge,
    NonFiniteSample,
    CleanupFailed {
        resource: &'static str,
        path: Option<PathBuf>,
        cause: CleanupFailure,
    },
    OperationAndCleanup {
        operation: Box<SpeechError>,
        cleanup: Box<SpeechError>,
    },
}

impl std::fmt::Display for SpeechError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TextTooLarge => write!(f, "speech text exceeded limit"),
            Self::Disabled => write!(f, "speech is disabled for this session"),
            Self::AlreadyActive => write!(f, "speech cleanup is still in progress"),
            Self::CreateTemp(_) => write!(f, "failed to create temporary speech output"),
            Self::InvalidTempPath => write!(f, "temporary speech path is not valid UTF-8"),
            Self::WorkerLaunch(_) => write!(f, "failed to launch speech worker"),
            Self::WorkerStartup(_) => write!(f, "speech worker failed to start"),
            Self::Synthesis(_) => write!(f, "speech synthesis failed"),
            Self::Playback(_) => write!(f, "speech playback failed"),
            Self::PlaybackDeviceFailed => write!(f, "audio output device reported an error"),
            Self::PlaybackTimedOut => write!(f, "speech playback timed out"),
            Self::PlaybackInitializationTimedOut => {
                write!(f, "audio output initialization timed out")
            }
            Self::PlaybackInitializationTaskFailed => {
                write!(f, "audio output initialization task failed")
            }
            Self::DecodeTimedOut => write!(f, "speech WAV decoding timed out"),
            Self::DecodeTaskFailed => write!(f, "speech WAV decode task failed"),
            Self::OpenWav(_) => write!(f, "failed to open synthesized WAV"),
            Self::DecodeWav(_) => write!(f, "failed to decode synthesized WAV"),
            Self::InvalidWavFormat => write!(f, "synthesized WAV has an invalid format"),
            Self::EmptyWav => write!(f, "synthesized WAV is empty"),
            Self::WavTooLong => write!(f, "synthesized WAV exceeded duration limit"),
            Self::WavTooLarge => write!(f, "synthesized WAV exceeded size limit"),
            Self::NonFiniteSample => write!(f, "synthesized WAV contained a non-finite sample"),
            Self::CleanupFailed {
                resource,
                path,
                cause,
            } => {
                write!(f, "failed to settle {resource}")?;
                if let Some(path) = path {
                    write!(f, " at {}", path.display())?;
                }
                match cause {
                    CleanupFailure::Worker(_) => write!(f, ": speech worker cleanup failed"),
                    CleanupFailure::TemporaryDirectory(_) => {
                        write!(f, ": temporary directory removal failed")
                    }
                    CleanupFailure::BlockingTaskTimeout => {
                        write!(f, ": blocking task did not finish before cleanup deadline")
                    }
                }
            }
            Self::OperationAndCleanup { operation, cleanup } => write!(f, "{operation}; {cleanup}"),
        }
    }
}

impl std::error::Error for SpeechError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::CreateTemp(error) => Some(error),
            Self::WorkerLaunch(error) | Self::WorkerStartup(error) | Self::Synthesis(error) => {
                Some(error)
            }
            Self::Playback(error) => Some(error),
            Self::OpenWav(error) | Self::DecodeWav(error) => Some(error),
            Self::CleanupFailed { cause, .. } => match cause {
                CleanupFailure::Worker(error) => Some(error),
                CleanupFailure::TemporaryDirectory(error) => Some(error),
                CleanupFailure::BlockingTaskTimeout => None,
            },
            Self::OperationAndCleanup { operation, .. } => Some(operation),
            _ => None,
        }
    }
}

pub struct SpeechOutput {
    config: TtsConfig,
    worker: Option<SupertonicWorker>,
    player: Option<AudioPlayer>,
    player_initialization: Option<JoinHandle<Result<AudioPlayer, PlaybackError>>>,
    active: Option<ActiveSpeech>,
    next_id: u64,
    cancelled_worker_restarts: u8,
    disabled: bool,
}

impl SpeechOutput {
    /// Stores the optional speech configuration without starting Python or opening audio.
    pub fn new(config: TtsConfig) -> Self {
        Self {
            config,
            worker: None,
            player: None,
            player_initialization: None,
            active: None,
            next_id: 1,
            cancelled_worker_restarts: 0,
            disabled: false,
        }
    }

    pub fn is_disabled(&self) -> bool {
        self.disabled
    }

    pub async fn speak(&mut self, text: &str) -> Result<(), SpeechError> {
        if text.len() > MAX_SPEECH_TEXT_BYTES {
            return Err(SpeechError::TextTooLarge);
        }
        if self.disabled {
            return Err(SpeechError::Disabled);
        }
        if self.active.is_some() {
            return Err(SpeechError::AlreadyActive);
        }

        self.ensure_player().await?;
        if self
            .player
            .as_ref()
            .is_some_and(AudioPlayer::device_has_failed)
        {
            self.disabled = true;
            return Err(SpeechError::PlaybackDeviceFailed);
        }

        let operation_dir = match Builder::new()
            .prefix("jarvis-speech-")
            .permissions(std::fs::Permissions::from_mode(0o700))
            .tempdir()
        {
            Ok(operation_dir) => operation_dir,
            Err(error) => {
                self.disabled = true;
                return Err(SpeechError::CreateTemp(error));
            }
        };
        let output_path = operation_dir.path().join("answer.wav");
        let output_path_string = match output_path.to_str() {
            Some(path) => path.to_owned(),
            None => {
                return Err(self.close_unowned_dir(operation_dir, SpeechError::InvalidTempPath));
            }
        };

        self.active = Some(ActiveSpeech {
            stage: SpeechStage::Starting,
            operation_dir: Some(operation_dir),
            decode_task: None,
        });

        let operation_id = self.next_operation_id();
        if self.worker.is_none() {
            let voice_style = self.config.model_dir.join("voice.bin");
            match SupertonicWorker::launch(
                &self.config.python,
                &self.config.worker_script,
                &self.config.model_dir,
                &voice_style,
                self.config.threads,
            ) {
                Ok(worker) => self.worker = Some(worker),
                Err(error) => {
                    return Err(self.fail_operation(SpeechError::WorkerLaunch(error)).await);
                }
            }
        }

        if self.worker.as_ref().is_some_and(|worker| !worker.is_idle())
            && let Err(error) = self
                .worker
                .as_mut()
                .expect("worker launched above")
                .wait_ready(self.config.threads)
                .await
        {
            return Err(self.fail_operation(SpeechError::WorkerStartup(error)).await);
        }

        self.active
            .as_mut()
            .expect("active operation directory was retained")
            .stage = SpeechStage::Synthesizing;

        if let Err(error) = self
            .worker
            .as_mut()
            .expect("worker is ready")
            .speak(operation_id, text.to_owned(), output_path_string)
            .await
        {
            return Err(self.fail_operation(SpeechError::Synthesis(error)).await);
        }

        self.active
            .as_mut()
            .expect("active operation directory was retained")
            .stage = SpeechStage::Decoding;
        let decode_path = output_path.clone();
        let decode_task = tokio::task::spawn_blocking(move || Self::decode_wav(&decode_path));
        self.active
            .as_mut()
            .expect("active operation directory was retained")
            .decode_task = Some(decode_task);

        let decoded = timeout(WAV_DECODE_TIMEOUT, self.await_decode()).await;
        let validated = match decoded {
            Err(_) => {
                self.disabled = true;
                return Err(SpeechError::DecodeTimedOut);
            }
            Ok(Ok(validated)) => validated,
            Ok(Err(operation)) => {
                self.disabled = true;
                let cleanup = self.close_active_directory();
                self.active = None;
                return Err(match cleanup {
                    Ok(()) => operation,
                    Err(cleanup) => Self::combine_operation_and_cleanup(operation, cleanup),
                });
            }
        };

        if let Err(cleanup) = self.close_active_directory() {
            self.active = None;
            self.disabled = true;
            return Err(cleanup);
        }

        let playback_deadline = validated.playback_deadline();
        if let Err(error) = self
            .player
            .as_mut()
            .expect("audio output initialized")
            .play(validated.samples)
        {
            self.disabled = true;
            self.active = None;
            return Err(SpeechError::Playback(error));
        }
        self.active
            .as_mut()
            .expect("active operation remains during playback")
            .stage = SpeechStage::Playing;

        let playback_deadline = tokio::time::Instant::now() + playback_deadline;
        loop {
            let player = self.player.as_ref().expect("audio output initialized");
            if player.device_has_failed() {
                self.player
                    .as_mut()
                    .expect("audio output initialized")
                    .stop();
                self.active = None;
                self.disabled = true;
                return Err(SpeechError::PlaybackDeviceFailed);
            }
            if !player.is_playing() {
                self.player
                    .as_mut()
                    .expect("audio output initialized")
                    .finish();
                self.active = None;
                return Ok(());
            }

            if tokio::time::Instant::now() >= playback_deadline {
                self.player
                    .as_mut()
                    .expect("audio output initialized")
                    .stop();
                self.active = None;
                self.disabled = true;
                return Err(SpeechError::PlaybackTimedOut);
            }

            tokio::time::sleep(PLAYBACK_POLL_INTERVAL).await;
        }
    }

    /// Cleans up work left behind after the `speak` future is dropped.
    pub async fn cancel(&mut self) -> Result<(), SpeechError> {
        match self.active.as_ref().map(|active| active.stage) {
            Some(SpeechStage::Starting | SpeechStage::Synthesizing) => {
                self.cancel_worker_operation().await
            }
            Some(SpeechStage::Decoding) => self.cancel_decode().await,
            Some(SpeechStage::Playing) => {
                if let Some(player) = self.player.as_mut() {
                    player.stop();
                }
                self.active = None;
                Ok(())
            }
            None => self.finish_pending_player_initialization().await,
        }
    }

    /// Shuts down the worker and releases persistent audio resources.
    pub async fn shutdown(&mut self) -> Result<(), SpeechError> {
        let mut shutdown_error = self.cancel().await.err();

        if let Some(worker) = self.worker.as_mut() {
            let result = if worker.is_idle() {
                worker.shutdown_idle().await
            } else {
                worker.terminate().await
            };
            if let Err(error) = result {
                self.disabled = true;
                let cleanup = SpeechError::CleanupFailed {
                    resource: "speech worker",
                    path: None,
                    cause: CleanupFailure::Worker(error),
                };
                shutdown_error = Some(match shutdown_error {
                    Some(operation) => Self::combine_operation_and_cleanup(operation, cleanup),
                    None => cleanup,
                });
            } else {
                self.worker = None;
            }
        }

        if self.worker.is_none()
            && self.active.as_ref().is_some_and(|active| {
                matches!(
                    active.stage,
                    SpeechStage::Starting | SpeechStage::Synthesizing
                )
            })
        {
            let cleanup = self.close_active_directory();
            self.active = None;
            if let Err(cleanup) = cleanup {
                self.disabled = true;
                shutdown_error = Some(match shutdown_error {
                    Some(operation) => Self::combine_operation_and_cleanup(operation, cleanup),
                    None => cleanup,
                });
            }
        }

        if let Some(player) = self.player.as_mut() {
            player.stop();
        }
        self.player = None;

        if self.player_initialization.is_some()
            && shutdown_error.is_none()
            && let Err(cleanup) = self.finish_pending_player_initialization().await
        {
            shutdown_error = Some(cleanup);
        }
        self.player = None;
        match shutdown_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    async fn ensure_player(&mut self) -> Result<(), SpeechError> {
        if self.player.is_some() {
            return Ok(());
        }
        if self.player_initialization.is_none() {
            let requested_device = self.config.audio_device.clone();
            self.player_initialization = Some(tokio::task::spawn_blocking(move || {
                AudioPlayer::new(requested_device)
            }));
        }

        let result = {
            let task = self
                .player_initialization
                .as_mut()
                .expect("player initialization was started");
            timeout(PLAYER_INITIALIZATION_TIMEOUT, &mut *task).await
        };

        match result {
            Err(_) => {
                self.disabled = true;
                Err(SpeechError::PlaybackInitializationTimedOut)
            }
            Ok(Ok(Ok(player))) => {
                self.player_initialization = None;
                eprintln!("speech output configuration: {:?}", player.output_config());
                self.player = Some(player);
                Ok(())
            }
            Ok(Ok(Err(error))) => {
                self.player_initialization = None;
                self.disabled = true;
                Err(SpeechError::Playback(error))
            }
            Ok(Err(_)) => {
                self.player_initialization = None;
                self.disabled = true;
                Err(SpeechError::PlaybackInitializationTaskFailed)
            }
        }
    }

    async fn finish_pending_player_initialization(&mut self) -> Result<(), SpeechError> {
        if self.player_initialization.is_none() {
            return Ok(());
        }

        let result = {
            let task = self
                .player_initialization
                .as_mut()
                .expect("checked player initialization task");
            timeout(CLEANUP_TIMEOUT, &mut *task).await
        };

        match result {
            Err(_) => {
                self.disabled = true;
                Err(SpeechError::CleanupFailed {
                    resource: "audio output initialization",
                    path: None,
                    cause: CleanupFailure::BlockingTaskTimeout,
                })
            }
            Ok(join_result) => {
                self.player_initialization = None;
                match join_result {
                    Ok(Ok(player)) => {
                        self.player = Some(player);
                        Ok(())
                    }
                    Ok(Err(error)) => {
                        self.disabled = true;
                        Err(SpeechError::Playback(error))
                    }
                    Err(_) => {
                        self.disabled = true;
                        Err(SpeechError::PlaybackInitializationTaskFailed)
                    }
                }
            }
        }
    }

    async fn await_decode(&mut self) -> Result<ValidatedWav, SpeechError> {
        let result = {
            let task = self
                .active
                .as_mut()
                .and_then(|active| active.decode_task.as_mut())
                .expect("decode task was stored before awaiting");
            (&mut *task).await
        };
        if let Some(active) = self.active.as_mut() {
            active.decode_task = None;
        }
        result.map_err(|_| SpeechError::DecodeTaskFailed)?
    }

    async fn cancel_worker_operation(&mut self) -> Result<(), SpeechError> {
        let path = self.active_directory_path();
        if let Some(worker) = self.worker.as_mut() {
            if let Err(error) = worker.terminate().await {
                self.disabled = true;
                return Err(SpeechError::CleanupFailed {
                    resource: "speech worker",
                    path,
                    cause: CleanupFailure::Worker(error),
                });
            }
            self.worker = None;

            if self.cancelled_worker_restarts == 0 {
                self.cancelled_worker_restarts = 1;
            } else {
                self.disabled = true;
            }
        }

        let cleanup = self.close_active_directory();
        self.active = None;
        if let Err(error) = cleanup {
            self.disabled = true;
            return Err(error);
        }
        Ok(())
    }

    async fn cancel_decode(&mut self) -> Result<(), SpeechError> {
        let decode_result = timeout(CLEANUP_TIMEOUT, self.await_decode()).await;
        let path = self.active_directory_path();
        match decode_result {
            Err(_) => {
                self.disabled = true;
                Err(SpeechError::CleanupFailed {
                    resource: "WAV decoder",
                    path,
                    cause: CleanupFailure::BlockingTaskTimeout,
                })
            }
            Ok(result) => {
                let cleanup = self.close_active_directory();
                self.active = None;
                match (result, cleanup) {
                    (Err(operation), Err(cleanup)) => {
                        self.disabled = true;
                        Err(Self::combine_operation_and_cleanup(operation, cleanup))
                    }
                    (Err(operation), Ok(())) => {
                        self.disabled = true;
                        Err(operation)
                    }
                    (Ok(_), Err(cleanup)) => {
                        self.disabled = true;
                        Err(cleanup)
                    }
                    (Ok(_), Ok(())) => Ok(()),
                }
            }
        }
    }

    async fn fail_operation(&mut self, operation: SpeechError) -> SpeechError {
        self.disabled = true;
        if let Some(worker) = self.worker.as_mut() {
            if let Err(error) = worker.terminate().await {
                return Self::combine_operation_and_cleanup(
                    operation,
                    SpeechError::CleanupFailed {
                        resource: "speech worker",
                        path: self.active_directory_path(),
                        cause: CleanupFailure::Worker(error),
                    },
                );
            }
            self.worker = None;
        }

        let cleanup = self.close_active_directory();
        self.active = None;
        match cleanup {
            Ok(()) => operation,
            Err(cleanup) => Self::combine_operation_and_cleanup(operation, cleanup),
        }
    }

    fn close_active_directory(&mut self) -> Result<(), SpeechError> {
        let Some(directory) = self
            .active
            .as_mut()
            .and_then(|active| active.operation_dir.take())
        else {
            return Ok(());
        };
        let path = directory.path().to_path_buf();
        directory
            .close()
            .map_err(|error| SpeechError::CleanupFailed {
                resource: "temporary speech directory",
                path: Some(path),
                cause: CleanupFailure::TemporaryDirectory(error),
            })
    }

    fn close_unowned_dir(&mut self, directory: TempDir, operation: SpeechError) -> SpeechError {
        self.disabled = true;
        let path = directory.path().to_path_buf();
        match directory.close() {
            Ok(()) => operation,
            Err(error) => Self::combine_operation_and_cleanup(
                operation,
                SpeechError::CleanupFailed {
                    resource: "temporary speech directory",
                    path: Some(path),
                    cause: CleanupFailure::TemporaryDirectory(error),
                },
            ),
        }
    }

    fn active_directory_path(&self) -> Option<PathBuf> {
        self.active
            .as_ref()
            .and_then(|active| active.operation_dir.as_ref())
            .map(|directory| directory.path().to_path_buf())
    }

    fn combine_operation_and_cleanup(operation: SpeechError, cleanup: SpeechError) -> SpeechError {
        SpeechError::OperationAndCleanup {
            operation: Box::new(operation),
            cleanup: Box::new(cleanup),
        }
    }

    fn next_operation_id(&mut self) -> String {
        let id = format!("speech-{}", self.next_id);
        self.next_id = self.next_id.saturating_add(1);
        id
    }

    fn decode_wav(path: &Path) -> Result<ValidatedWav, SpeechError> {
        let metadata = std::fs::metadata(path)
            .map_err(|error| SpeechError::OpenWav(hound::Error::IoError(error)))?;
        if metadata.len() > MAX_WAV_BYTES {
            return Err(SpeechError::WavTooLarge);
        }

        let mut reader = hound::WavReader::open(path).map_err(SpeechError::OpenWav)?;
        let spec = reader.spec();
        if spec.channels != EXPECTED_CHANNELS
            || spec.sample_rate != EXPECTED_SAMPLE_RATE
            || spec.bits_per_sample != EXPECTED_BITS_PER_SAMPLE
            || spec.sample_format != hound::SampleFormat::Int
        {
            return Err(SpeechError::InvalidWavFormat);
        }

        let declared_samples = reader.duration() as usize;
        if declared_samples == 0 {
            return Err(SpeechError::EmptyWav);
        }
        if declared_samples > MAX_WAV_SAMPLES {
            return Err(SpeechError::WavTooLong);
        }

        let mut samples = Vec::with_capacity(declared_samples);
        for sample in reader.samples::<i16>() {
            if samples.len() == MAX_WAV_SAMPLES {
                return Err(SpeechError::WavTooLong);
            }
            let sample = sample.map_err(SpeechError::DecodeWav)?;
            let sample = sample as f32 / 32_768.0;
            if !sample.is_finite() {
                return Err(SpeechError::NonFiniteSample);
            }
            samples.push(sample);
        }

        if samples.is_empty() {
            return Err(SpeechError::EmptyWav);
        }

        let duration = Duration::from_secs_f64(samples.len() as f64 / EXPECTED_SAMPLE_RATE as f64);
        Ok(ValidatedWav { samples, duration })
    }
}

impl Drop for SpeechOutput {
    fn drop(&mut self) {
        if let Some(active) = self.active.take()
            && let Some(directory) = active.operation_dir
        {
            let path = directory.keep();
            eprintln!(
                "warning: preserving unresolved speech output directory {}",
                path.display()
            );
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/services/tts/speech_tests.rs"]
mod tests;
