//! How loud the speech being played is, 0 to 1, measured on the samples rodio's mixer pulls.
//!
//! rodio 0.22.2 has no adapter that shows a source's samples to user code: `periodic_access`
//! (`source/periodic.rs`) hands its closure the inner source, not the sample, and there is no
//! `inspect`. So [`LevelTap`] is a small `Source` wrapper in the style of `PeriodicAccess`.
//! It sits between each `SamplesBuffer` and the `Player`, so it is pulled by the mixer at
//! playback pace, ahead of rodio's resampler. A tap is dropped with its source when the
//! queue is stopped.
//!
//! The level is RMS over windows of [`LEVEL_UPDATES_PER_SECOND`] per second of *played
//! audio* (a window is `sample_rate / 30` samples, about 33 ms). Counting samples rather than
//! wall time keeps the level in step with the audio. The display scale is the reader's job:
//! speech RMS is usually well below 1.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use rodio::{ChannelCount, SampleRate, Source, source::SeekError};
use tokio::sync::watch;

/// How many times a second a new level is published while audio plays (about every 33 ms).
const LEVEL_UPDATES_PER_SECOND: u32 = 30;

/// Level when nothing is playing, and the value after playback ends or is stopped.
const SILENT_LEVEL: f32 = 0.0;

/// The publishing side of the level, owned by the audio player.
///
/// Playback is split into sessions (one answer each). [`PlaybackLevel::end`] closes the
/// current session: its taps stop publishing and the level goes to 0, so audio still in
/// flight on the audio thread cannot bring a stale level back.
pub(super) struct PlaybackLevel {
    level: watch::Sender<f32>,
    /// True while the current session may publish. Replaced by [`PlaybackLevel::end`].
    session_live: Arc<AtomicBool>,
}

impl PlaybackLevel {
    pub fn new() -> Self {
        Self {
            level: watch::Sender::new(SILENT_LEVEL),
            session_live: Arc::new(AtomicBool::new(true)),
        }
    }

    pub fn subscribe(&self) -> LevelReceiver {
        LevelReceiver {
            level: self.level.subscribe(),
        }
    }

    /// Wraps `source` so its samples are measured as they are played.
    pub fn tap<S: Source>(&self, source: S) -> LevelTap<S> {
        LevelTap {
            window: LevelWindow::new(window_length(source.sample_rate(), source.channels())),
            inner: source,
            level: self.level.clone(),
            session_live: Arc::clone(&self.session_live),
        }
    }

    /// Ends the current session: the level goes to 0 and nothing more is published for
    /// audio queued so far. Taps made after this call start a new session.
    pub fn end(&mut self) {
        self.session_live.store(false, Ordering::Release);
        // `send_replace` takes the same lock as a tap's publish, so a tap either published
        // before this (and is overwritten) or sees the closed session and publishes nothing.
        self.level.send_replace(SILENT_LEVEL);
        self.session_live = Arc::new(AtomicBool::new(true));
    }
}

/// Reads the playback level. Cheap to clone.
///
/// It offers copies, never a borrow of the shared value: a held `watch::Ref` would block
/// the audio thread's next publish, and a slow or absent reader must not do that.
#[derive(Clone, Debug)]
pub(crate) struct LevelReceiver {
    level: watch::Receiver<f32>,
}

// The reader is the orb interface (visual-jarvis ticket 07).
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "read by visual-jarvis ticket 07")
)]
impl LevelReceiver {
    /// The latest level, 0.0 (silence or nothing playing) to 1.0 (full scale).
    pub fn level(&self) -> f32 {
        *self.level.borrow()
    }

    /// Waits for a level newer than the last one this receiver saw, then returns it.
    ///
    /// Updates are coalesced: a reader that falls behind sees the latest level, not a backlog.
    /// Errors once the player is gone; the level was reset to 0 first.
    pub async fn changed(&mut self) -> Result<f32, watch::error::RecvError> {
        self.level.changed().await?;
        Ok(*self.level.borrow_and_update())
    }
}

/// A `Source` that measures the loudness of the samples passing through it.
///
/// It runs on the audio thread. Per sample it does a few float operations; it never
/// allocates or waits on the reader. Publishing once per window briefly takes the watch's
/// internal lock, which a [`LevelReceiver`] holds only while copying one `f32`.
///
/// The window restarts with each source, so a source's last partial window (under 33 ms)
/// is not reported.
pub(super) struct LevelTap<S> {
    inner: S,
    window: LevelWindow,
    level: watch::Sender<f32>,
    session_live: Arc<AtomicBool>,
}

impl<S: Source> Iterator for LevelTap<S> {
    type Item = S::Item;

    fn next(&mut self) -> Option<S::Item> {
        let sample = self.inner.next()?;
        if let Some(level) = self.window.push(sample) {
            self.level.send_if_modified(|current| {
                // Checked under the watch's lock, so `PlaybackLevel::end` cannot interleave.
                if !self.session_live.load(Ordering::Acquire) {
                    return false;
                }
                *current = level;
                true
            });
        }
        Some(sample)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl<S: Source> Source for LevelTap<S> {
    fn current_span_len(&self) -> Option<usize> {
        self.inner.current_span_len()
    }

    fn channels(&self) -> ChannelCount {
        self.inner.channels()
    }

    fn sample_rate(&self) -> SampleRate {
        self.inner.sample_rate()
    }

    fn total_duration(&self) -> Option<Duration> {
        self.inner.total_duration()
    }

    fn try_seek(&mut self, pos: Duration) -> Result<(), SeekError> {
        self.inner.try_seek(pos)
    }
}

/// Samples (all channels) in one update window of a source with this format.
///
/// Fixed when the tap is made, as rodio's own `PeriodicAccess` does; a speech chunk
/// never changes rate. At least 1, so a window always closes.
fn window_length(sample_rate: SampleRate, channels: ChannelCount) -> u32 {
    (sample_rate.get().saturating_mul(u32::from(channels.get())) / LEVEL_UPDATES_PER_SECOND).max(1)
}

/// Running RMS over fixed-length windows of samples.
struct LevelWindow {
    length: u32,
    filled: u32,
    sum_of_squares: f32,
}

impl LevelWindow {
    fn new(length: u32) -> Self {
        Self {
            length,
            filled: 0,
            sum_of_squares: 0.0,
        }
    }

    /// Adds one sample; returns the window's level when this sample completes it.
    ///
    /// The level is the root mean square of the window: 0.0 for silence and 1.0 for a
    /// full-scale signal. Samples are clipped to -1..=1, as the output stage does, and a
    /// non-finite sample counts as silence.
    fn push(&mut self, sample: f32) -> Option<f32> {
        let sample = if sample.is_finite() {
            sample.clamp(-1.0, 1.0)
        } else {
            0.0
        };
        self.sum_of_squares += sample * sample;
        self.filled += 1;
        if self.filled < self.length {
            return None;
        }

        // A window is one 30th of a second of samples (at most 1,600 per channel at the
        // 48 kHz limit), far below 2^24, so the `f32` conversion is exact.
        let mean = self.sum_of_squares / self.length as f32;
        self.filled = 0;
        self.sum_of_squares = 0.0;
        Some(mean.sqrt().min(1.0))
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/features/speech/playback_level_tests.rs"]
mod tests;
