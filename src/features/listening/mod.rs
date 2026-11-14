//! Microphone capture through the `arecord` CLI.
//!
//! Audio stays in memory: `arecord` writes raw PCM to its stdout and nothing
//! is stored on disk.

use std::{
    io,
    process::{ExitStatus, Stdio},
    sync::Arc,
    time::Duration,
};

use thiserror::Error;
use tokio::{
    io::AsyncReadExt,
    process::Command,
    sync::{broadcast, oneshot},
    time::Instant,
};

mod level;
mod listener;
mod vad;

pub(crate) use level::{LEVEL_INTERVAL, run_level_meter};
pub(crate) use listener::{Heard, ListenError, Listener};
pub(crate) use vad::speech_ended;

const SAMPLE_RATE_HZ: u32 = 16_000;
const CHANNELS: u32 = 1;
const BYTES_PER_SAMPLE: u32 = 2;
const BYTES_PER_SECOND: u32 = SAMPLE_RATE_HZ * CHANNELS * BYTES_PER_SAMPLE;
/// The longest recording in seconds; a front end can tell the user.
pub(crate) const MAX_RECORDING_SECONDS: u32 = 15;
const MIN_RECORDING_MILLISECONDS: u32 = 300;
const MILLISECONDS_PER_SECOND: u32 = 1000;
// The `as` casts below widen u32 values that are at most
// 16_000 * 2 * 15 = 480_000, so they cannot truncate on any target.
/// Recording stops here even if the caller has not asked it to: 15 s of PCM.
const MAX_RECORDING_BYTES: usize = (BYTES_PER_SECOND * MAX_RECORDING_SECONDS) as usize;
/// Shorter recordings are rejected as accidental: 0.3 s of PCM.
const MIN_RECORDING_BYTES: usize =
    (BYTES_PER_SECOND * MIN_RECORDING_MILLISECONDS / MILLISECONDS_PER_SECOND) as usize;
/// Grace on top of the recording cap for the recorder's start-up latency.
const RECORDING_WAIT_GRACE: Duration = Duration::from_secs(1);
/// Wall-clock limit for one recording, so a recorder that never sends audio
/// (device busy, wrong card) cannot wait for `stop` forever.
const MAX_RECORDING_WAIT: Duration =
    Duration::from_secs(MAX_RECORDING_SECONDS as u64).saturating_add(RECORDING_WAIT_GRACE);
const READ_CHUNK_BYTES: usize = 4096;
/// Chunks an observer may fall behind before it loses the oldest: 64 reads, up to
/// 8 s of audio.
const TAP_CAPACITY_CHUNKS: usize = 64;

/// A slice of the recorder's audio, shared by every observer: S16_LE PCM, always a
/// whole number of samples (an even number of bytes), so it can be decoded alone.
pub(crate) type AudioChunk = Arc<[u8]>;

#[derive(Debug, Error)]
pub(crate) enum CaptureError {
    #[error("failed to start the recorder")]
    Spawn(#[source] io::Error),
    #[error("recorder stdout unavailable")]
    MissingStdout,
    #[error("failed to read from the recorder")]
    Read(#[source] io::Error),
    #[error("failed to stop the recorder")]
    Stop(#[source] io::Error),
    #[error("recorder exited with {0}")]
    Failed(ExitStatus),
    #[error("recording too short: {bytes} bytes")]
    TooShort { bytes: usize },
}

/// Mono 16 kHz S16_LE PCM held in memory. `pcm` always holds whole samples.
#[derive(Debug)]
pub(crate) struct Recording {
    pub(crate) pcm: Vec<u8>,
}

/// Records from one ALSA device by running a recorder program.
pub(crate) struct Recorder {
    device: String,
    program: String,
    max_wait: Duration,
    /// Every chunk read, for observers; see `Recorder::tap`.
    tap: broadcast::Sender<AudioChunk>,
}

impl Recorder {
    /// `program` is `"arecord"` in production; tests pass a fake script.
    pub(crate) fn new(device: impl Into<String>, program: impl Into<String>) -> Self {
        Self {
            device: device.into(),
            program: program.into(),
            max_wait: MAX_RECORDING_WAIT,
            tap: broadcast::channel(TAP_CAPACITY_CHUNKS).0,
        }
    }

    /// A new observer of the audio, such as a level meter or a voice-activity
    /// detector. Each call is an independent observer, so any number can watch one
    /// recording without `record` knowing about them.
    ///
    /// It sees every chunk of every later `record` call, in order, as the chunk is
    /// read. Observers can never slow or end a recording: the tap is a bounded
    /// broadcast, so one that falls `TAP_CAPACITY_CHUNKS` behind loses the oldest
    /// chunks (its next `recv` reports `Lagged`), and a dropped one is just not
    /// sent to. Chunks are sample-aligned: each holds a whole number of samples,
    /// so one never starts or ends mid-sample, even when a read does (the half
    /// sample joins the next chunk), and a lone byte at the end of a recording is
    /// never sent. The last chunk may carry bytes past `MAX_RECORDING_BYTES` that
    /// `Recording` discards. An observer that subscribes mid-recording starts at
    /// the next sample boundary; one that lags loses whole chunks, so it stays
    /// aligned.
    pub(crate) fn tap(&self) -> broadcast::Receiver<AudioChunk> {
        self.tap.subscribe()
    }

    /// `recording_started` is sent when the first audio arrives, the earliest moment
    /// that speaking is certain to be captured; it is dropped unsent when no audio
    /// ever arrives.
    ///
    /// Records until `stop` completes, the recorder exits,
    /// `MAX_RECORDING_BYTES` have been read (the excess is never returned), or
    /// `MAX_RECORDING_WAIT` elapses. The deadline is handled like an early
    /// stop: the child is killed and what was read is returned under the same
    /// min/max rules, so a recorder that never sends audio gives `TooShort`.
    ///
    /// Safe to drop or race in `select!` at any `.await`: the child is spawned
    /// with `kill_on_drop`, and the normal stop path kills and reaps it
    /// explicitly, so no `arecord` process is left running or zombied. A
    /// trailing odd byte (half a sample) is discarded.
    pub(crate) async fn record(
        &self,
        stop: impl Future<Output = ()>,
        recording_started: oneshot::Sender<()>,
    ) -> Result<Recording, CaptureError> {
        let mut child = Command::new(&self.program)
            .args(["-q", "-D", &self.device])
            .args(["-f", "S16_LE"])
            .args(["-r", &SAMPLE_RATE_HZ.to_string()])
            .args(["-c", &CHANNELS.to_string()])
            .args(["-t", "raw"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(CaptureError::Spawn)?;
        let mut stdout = child.stdout.take().ok_or(CaptureError::MissingStdout)?;

        let mut pcm: Vec<u8> = Vec::new();
        // How much of `pcm` has been offered to the tap: always a whole number of samples.
        let mut tapped = 0usize;
        let mut chunk = [0u8; READ_CHUNK_BYTES];
        let mut recorder_ended = false;
        let mut recording_started = Some(recording_started);
        let deadline = tokio::time::sleep_until(Instant::now() + self.max_wait);
        tokio::pin!(stop, deadline);
        while pcm.len() < MAX_RECORDING_BYTES {
            let read = tokio::select! {
                biased;
                () = &mut stop => break,
                () = &mut deadline => break,
                read = stdout.read(&mut chunk) => read,
            };
            match read {
                Ok(0) => {
                    recorder_ended = true;
                    break;
                }
                Ok(count) => {
                    pcm.extend_from_slice(&chunk[..count]);
                    // Tap only whole samples: a read can end mid-sample, and the half
                    // sample waits in `pcm` for the next read. `tapped` advances even
                    // when nobody watches, so an observer that subscribes mid-recording
                    // starts on a sample boundary and is never sent old audio. A final
                    // lone byte is never tapped. `send` never waits; it fails only
                    // without receivers, which is fine, and the copy is skipped then.
                    let aligned = pcm.len() & !1;
                    if aligned > tapped && self.tap.receiver_count() > 0 {
                        let _ = self.tap.send(Arc::from(&pcm[tapped..aligned]));
                    }
                    tapped = aligned;
                    // A receiver that is gone just means nobody is showing the stage.
                    if let Some(started) = recording_started.take() {
                        let _ = started.send(());
                    }
                }
                Err(error) => {
                    let _ = child.kill().await;
                    return Err(CaptureError::Read(error));
                }
            }
        }

        if recorder_ended {
            let status = child.wait().await.map_err(CaptureError::Stop)?;
            if !status.success() {
                return Err(CaptureError::Failed(status));
            }
        } else {
            // We are ending it, so its killed exit status is expected.
            child.kill().await.map_err(CaptureError::Stop)?;
        }

        pcm.truncate(MAX_RECORDING_BYTES.min(pcm.len() & !1));
        if pcm.len() < MIN_RECORDING_BYTES {
            return Err(CaptureError::TooShort { bytes: pcm.len() });
        }
        Ok(Recording { pcm })
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/features/listening/listening_tests.rs"]
mod tests;
