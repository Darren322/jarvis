use std::{
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::Duration,
};

use tempfile::{Builder, TempDir};
use thiserror::Error;
use tokio::{task::JoinHandle, time::timeout};

use crate::config::TtsConfig;

use super::{
    playback::{AudioPlayer, PlaybackError, WAKE_NOISE_DURATION},
    supertonic_worker::{SupertonicWorker, WorkerError},
};

/// Worker failures in a row that disable speech for the session; a success resets the count.
const MAX_CONSECUTIVE_SPEECH_FAILURES: u8 = 3;
const MAX_SPEECH_TEXT_BYTES: usize = 4 * 1024;
const EXPECTED_SAMPLE_RATE: u32 = 44_100;
const EXPECTED_CHANNELS: u16 = 1;
const EXPECTED_BITS_PER_SAMPLE: u16 = 16;
const MAX_WAV_SAMPLES: usize = 5_292_000;
const MAX_WAV_BYTES: u64 = 16 * 1024 * 1024;
const PLAYBACK_GRACE: Duration = Duration::from_secs(2);
const MAX_PLAYBACK_DURATION: Duration = Duration::from_millis(122_700);
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
            .saturating_add(WAKE_NOISE_DURATION)
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

#[derive(Debug, Error)]
pub(crate) enum CleanupFailure {
    #[error("speech worker cleanup failed")]
    Worker(#[source] WorkerError),
    #[error("temporary directory removal failed")]
    TemporaryDirectory(#[source] std::io::Error),
    #[error("blocking task did not finish before cleanup deadline")]
    BlockingTaskTimeout,
}

/// Why speech stopped for the rest of the session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub(crate) enum DisabledReason {
    #[error("speech failed {MAX_CONSECUTIVE_SPEECH_FAILURES} times in a row")]
    ConsecutiveFailures,
    /// The audio device, a temporary file or cleanup failed in a way that cannot be retried safely.
    #[error("speech hit an unrecoverable audio or cleanup error")]
    Unrecoverable,
}

/// Whether the next answer may be spoken.
///
/// A worker failure moves `Ready` to `Recovering`; the next answer starts a fresh worker.
/// `Disabled` is terminal for the session and keeps its first reason.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SpeechHealth {
    Ready,
    Recovering { consecutive_failures: u8 },
    Disabled(DisabledReason),
}

impl SpeechHealth {
    fn record_failure(&mut self) {
        *self = match *self {
            Self::Ready => Self::after_failures(1),
            Self::Recovering {
                consecutive_failures,
            } => Self::after_failures(consecutive_failures.saturating_add(1)),
            Self::Disabled(reason) => Self::Disabled(reason),
        };
    }

    fn after_failures(consecutive_failures: u8) -> Self {
        if consecutive_failures >= MAX_CONSECUTIVE_SPEECH_FAILURES {
            Self::Disabled(DisabledReason::ConsecutiveFailures)
        } else {
            Self::Recovering {
                consecutive_failures,
            }
        }
    }

    fn record_success(&mut self) {
        match *self {
            Self::Ready | Self::Recovering { .. } => *self = Self::Ready,
            Self::Disabled(_) => {}
        }
    }

    fn disable(&mut self) {
        match *self {
            Self::Ready | Self::Recovering { .. } => {
                *self = Self::Disabled(DisabledReason::Unrecoverable);
            }
            Self::Disabled(_) => {}
        }
    }
}

#[derive(Debug, Error)]
pub(crate) enum SpeechError {
    #[error("speech text exceeded limit")]
    TextTooLarge,
    #[error("speech is disabled for this session")]
    Disabled(#[source] DisabledReason),
    #[error("speech cleanup is still in progress")]
    AlreadyActive,
    #[error("failed to create temporary speech output")]
    CreateTemp(#[source] std::io::Error),
    #[error("temporary speech path is not valid utf-8")]
    InvalidTempPath,
    #[error("failed to launch speech worker")]
    WorkerLaunch(#[source] WorkerError),
    #[error("speech worker failed to start")]
    WorkerStartup(#[source] WorkerError),
    #[error("speech synthesis failed")]
    Synthesis(#[source] WorkerError),
    #[error("speech playback failed")]
    Playback(#[source] PlaybackError),
    #[error("audio output device reported an error")]
    PlaybackDeviceFailed,
    #[error("speech playback timed out")]
    PlaybackTimedOut,
    #[error("audio output initialization timed out")]
    PlaybackInitializationTimedOut,
    #[error("audio output initialization task failed")]
    PlaybackInitializationTaskFailed,
    #[error("speech wav decoding timed out")]
    DecodeTimedOut,
    #[error("speech wav decode task failed")]
    DecodeTaskFailed,
    #[error("failed to open synthesized wav")]
    OpenWav(#[source] hound::Error),
    #[error("failed to decode synthesized wav")]
    DecodeWav(#[source] hound::Error),
    #[error("synthesized wav has an invalid format")]
    InvalidWavFormat,
    #[error("synthesized wav is empty")]
    EmptyWav,
    #[error("synthesized wav exceeded duration limit")]
    WavTooLong,
    #[error("synthesized wav exceeded size limit")]
    WavTooLarge,
    #[error("synthesized wav contained a non-finite sample")]
    NonFiniteSample,
    #[error("failed to settle {resource}{}", display_path_suffix(path))]
    CleanupFailed {
        resource: &'static str,
        path: Option<PathBuf>,
        #[source]
        cause: CleanupFailure,
    },
    #[error("speech cleanup also failed: {cleanup}")]
    OperationAndCleanup {
        #[source]
        operation: Box<SpeechError>,
        cleanup: Box<SpeechError>,
    },
}

fn display_path_suffix(path: &Option<PathBuf>) -> String {
    path.as_ref()
        .map(|path| format!(" at {}", path.display()))
        .unwrap_or_default()
}

pub(crate) struct SpeechOutput {
    config: TtsConfig,
    worker: Option<SupertonicWorker>,
    player: Option<AudioPlayer>,
    player_initialization: Option<JoinHandle<Result<AudioPlayer, PlaybackError>>>,
    active: Option<ActiveSpeech>,
    next_id: u64,
    health: SpeechHealth,
}

impl SpeechOutput {
    /// Stores the optional speech configuration without starting Python or opening audio.
    pub(crate) fn new(config: TtsConfig) -> Self {
        Self {
            config,
            worker: None,
            player: None,
            player_initialization: None,
            active: None,
            next_id: 1,
            health: SpeechHealth::Ready,
        }
    }

    pub(crate) fn is_disabled(&self) -> bool {
        self.disabled_reason().is_some()
    }

    /// Why speech is off for the rest of the session, if it is.
    pub(crate) fn disabled_reason(&self) -> Option<DisabledReason> {
        match self.health {
            SpeechHealth::Ready | SpeechHealth::Recovering { .. } => None,
            SpeechHealth::Disabled(reason) => Some(reason),
        }
    }

    fn ensure_accepting(&self) -> Result<(), SpeechError> {
        match self.health {
            SpeechHealth::Disabled(reason) => return Err(SpeechError::Disabled(reason)),
            SpeechHealth::Ready | SpeechHealth::Recovering { .. } => {}
        }
        if self.active.is_some() {
            return Err(SpeechError::AlreadyActive);
        }
        Ok(())
    }

    pub(crate) async fn speak(
        &mut self,
        text: &str,
        playback_started: tokio::sync::oneshot::Sender<()>,
    ) -> Result<(), SpeechError> {
        if text.len() > MAX_SPEECH_TEXT_BYTES {
            return Err(SpeechError::TextTooLarge);
        }
        self.ensure_accepting()?;

        self.ensure_player().await?;
        if self
            .player
            .as_ref()
            .is_some_and(AudioPlayer::has_device_failed)
        {
            self.count_failure_and_reset_player();
            return Err(SpeechError::PlaybackDeviceFailed);
        }

        let operation_dir = match Builder::new()
            .prefix("jarvis-speech-")
            .permissions(std::fs::Permissions::from_mode(0o700))
            .tempdir()
        {
            Ok(operation_dir) => operation_dir,
            Err(error) => {
                self.health.record_failure();
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
                self.health.record_failure();
                // The blocking decoder cannot be interrupted; settle it like a cancel so the
                // directory is released, or disable speech if it will not finish.
                return Err(match self.cancel_decode().await {
                    Ok(()) => SpeechError::DecodeTimedOut,
                    Err(cleanup) => {
                        Self::combine_operation_and_cleanup(SpeechError::DecodeTimedOut, cleanup)
                    }
                });
            }
            Ok(Ok(validated)) => validated,
            Ok(Err(operation)) => {
                self.health.record_failure();
                let cleanup = self.close_active_directory();
                self.active = None;
                return Err(match cleanup {
                    Ok(()) => operation,
                    Err(cleanup) => {
                        self.health.disable();
                        Self::combine_operation_and_cleanup(operation, cleanup)
                    }
                });
            }
        };

        if let Err(cleanup) = self.close_active_directory() {
            self.active = None;
            self.health.disable();
            return Err(cleanup);
        }

        let playback_deadline = validated.playback_deadline();
        if let Err(error) = self
            .player
            .as_mut()
            .expect("audio output initialized")
            .play(validated.samples)
        {
            self.active = None;
            self.count_failure_and_reset_player();
            return Err(SpeechError::Playback(error));
        }
        self.active
            .as_mut()
            .expect("active operation remains during playback")
            .stage = SpeechStage::Playing;
        let _ = playback_started.send(());

        let playback_deadline = tokio::time::Instant::now() + playback_deadline;
        loop {
            let player = self.player.as_ref().expect("audio output initialized");
            if player.has_device_failed() {
                self.player
                    .as_mut()
                    .expect("audio output initialized")
                    .stop();
                self.active = None;
                self.count_failure_and_reset_player();
                return Err(SpeechError::PlaybackDeviceFailed);
            }
            if !player.is_playing() {
                self.player
                    .as_mut()
                    .expect("audio output initialized")
                    .finish();
                self.active = None;
                self.health.record_success();
                return Ok(());
            }

            if tokio::time::Instant::now() >= playback_deadline {
                self.player
                    .as_mut()
                    .expect("audio output initialized")
                    .stop();
                self.active = None;
                self.count_failure_and_reset_player();
                return Err(SpeechError::PlaybackTimedOut);
            }

            tokio::time::sleep(PLAYBACK_POLL_INTERVAL).await;
        }
    }

    /// Cleans up work left behind after the `speak` future is dropped.
    pub(crate) async fn cancel(&mut self) -> Result<(), SpeechError> {
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
    pub(crate) async fn shutdown(&mut self) -> Result<(), SpeechError> {
        let mut shutdown_error = self.cancel().await.err();

        if let Some(worker) = self.worker.as_mut() {
            let result = if worker.is_idle() {
                worker.shutdown_idle().await
            } else {
                worker.terminate().await
            };
            if let Err(error) = result {
                self.health.disable();
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
                self.health.disable();
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
                // The blocking task stays owned in `player_initialization`; the next answer
                // waits for it again.
                self.health.record_failure();
                Err(SpeechError::PlaybackInitializationTimedOut)
            }
            Ok(Ok(Ok(player))) => {
                self.player_initialization = None;
                self.player = Some(player);
                Ok(())
            }
            Ok(Ok(Err(error))) => {
                self.player_initialization = None;
                self.health.record_failure();
                Err(SpeechError::Playback(error))
            }
            Ok(Err(_)) => {
                self.player_initialization = None;
                self.health.record_failure();
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
                self.health.disable();
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
                    Ok(Err(error)) => Err(SpeechError::Playback(error)),
                    Err(_) => Err(SpeechError::PlaybackInitializationTaskFailed),
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
                self.health.disable();
                return Err(SpeechError::CleanupFailed {
                    resource: "speech worker",
                    path,
                    cause: CleanupFailure::Worker(error),
                });
            }
            self.worker = None;
        }

        let cleanup = self.close_active_directory();
        self.active = None;
        if let Err(error) = cleanup {
            self.health.disable();
            return Err(error);
        }
        Ok(())
    }

    async fn cancel_decode(&mut self) -> Result<(), SpeechError> {
        let decode_result = timeout(CLEANUP_TIMEOUT, self.await_decode()).await;
        let path = self.active_directory_path();
        match decode_result {
            Err(_) => {
                self.health.disable();
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
                        self.health.disable();
                        Err(Self::combine_operation_and_cleanup(operation, cleanup))
                    }
                    (Err(operation), Ok(())) => Err(operation),
                    (Ok(_), Err(cleanup)) => {
                        self.health.disable();
                        Err(cleanup)
                    }
                    (Ok(_), Ok(())) => Ok(()),
                }
            }
        }
    }

    /// Terminates the worker and cleans up after a failed answer.
    ///
    /// The failure is counted; the next answer starts a fresh worker unless this was the
    /// last allowed one. If cleanup itself fails, speech is disabled because resources
    /// may still be held.
    async fn fail_operation(&mut self, operation: SpeechError) -> SpeechError {
        self.health.record_failure();
        if let Some(worker) = self.worker.as_mut() {
            if let Err(error) = worker.terminate().await {
                self.health.disable();
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
            Err(cleanup) => {
                self.health.disable();
                Self::combine_operation_and_cleanup(operation, cleanup)
            }
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
        self.health.record_failure();
        let path = directory.path().to_path_buf();
        match directory.close() {
            Ok(()) => operation,
            Err(error) => {
                self.health.disable();
                Self::combine_operation_and_cleanup(
                    operation,
                    SpeechError::CleanupFailed {
                        resource: "temporary speech directory",
                        path: Some(path),
                        cause: CleanupFailure::TemporaryDirectory(error),
                    },
                )
            }
        }
    }

    /// Counts a failed answer and drops the audio player so the next answer opens the
    /// device again; a USB device can glitch and recover.
    fn count_failure_and_reset_player(&mut self) {
        if let Some(player) = self.player.as_mut() {
            player.stop();
        }
        self.player = None;
        self.health.record_failure();
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
#[path = "../../../tests/unit/features/speech/output_tests.rs"]
mod tests;
