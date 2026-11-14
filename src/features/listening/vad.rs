//! Ends a recording when the speaker stops talking.
//!
//! `SpeechEnd` is the pure rule (speech confirmed, then enough silence);
//! `speech_ended` feeds it with earshot's per-frame decisions on the recorder's
//! audio tap, as a plain future that a caller passes as `stop`.

use std::future::pending;

use earshot::Detector;
use tokio::sync::broadcast::{self, error::RecvError};

use super::AudioChunk;

/// Samples in one detector frame; earshot accepts exactly this many at 16 kHz.
const FRAME_SAMPLES: usize = 256;
/// Length of one detector frame: 256 samples at 16 kHz.
const FRAME_MILLISECONDS: u32 = 16;
/// earshot scores of at least this value count as speech (its documented default).
const SPEECH_SCORE: f32 = 0.5;
/// Silence after speech that ends the recording, in milliseconds.
const SILENCE_LENGTH_MILLISECONDS: u32 = 1000;
/// `SILENCE_LENGTH_MILLISECONDS` in whole frames, rounded up: 63 frames, 1008 ms.
const SILENCE_FRAMES: u32 = SILENCE_LENGTH_MILLISECONDS.div_ceil(FRAME_MILLISECONDS);
/// Consecutive speech frames before speech counts as heard: 6 frames, 96 ms.
///
/// On real clips earshot flagged an isolated 16 ms frame of background noise,
/// and on synthetic noise the first frame or two, so one frame must not arm
/// the stop; about 100 ms of speech is shorter than any word.
const MIN_SPEECH_FRAMES: u32 = 6;

/// Where a recording is in the speech-then-silence rule. Silence alone never
/// ends a recording: only `Speaking` can.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SpeechEnd {
    /// No speech confirmed yet; `speech_run` consecutive speech frames so far.
    Waiting { speech_run: u32 },
    /// Speech was confirmed; `silence_run` consecutive non-speech frames since.
    Speaking { silence_run: u32 },
}

impl SpeechEnd {
    const fn new() -> Self {
        Self::Waiting { speech_run: 0 }
    }

    /// Takes the next frame's decision; true when the recording should end now.
    ///
    /// Any speech frame resets the silence run, so a short pause inside a
    /// sentence does not end it. After a true result the caller stops asking.
    fn push(&mut self, is_speech: bool) -> bool {
        match *self {
            Self::Waiting { speech_run } => {
                let speech_run = if is_speech { speech_run + 1 } else { 0 };
                *self = if speech_run >= MIN_SPEECH_FRAMES {
                    Self::Speaking { silence_run: 0 }
                } else {
                    Self::Waiting { speech_run }
                };
                false
            }
            Self::Speaking { silence_run } => {
                let silence_run = if is_speech { 0 } else { silence_run + 1 };
                *self = Self::Speaking { silence_run };
                silence_run >= SILENCE_FRAMES
            }
        }
    }
}

/// Completes once speech has been heard on the tap and then followed by
/// `SILENCE_LENGTH_MILLISECONDS` of silence.
///
/// It never completes on silence alone, and never when the tap closes or lags:
/// the recorder's own caps end those recordings. A lagged tap loses whole chunks,
/// so the partial frame is dropped and framing stays sample-aligned. It owns only
/// its receiver, detector and carry buffer, so it is safe to drop at any
/// `.await`. Pass it as `stop` to `Recorder::record`.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "wired by visual-jarvis ticket 07")
)]
pub(crate) async fn speech_ended(mut chunks: broadcast::Receiver<AudioChunk>) {
    let mut detector = Detector::default_boxed();
    let mut state = SpeechEnd::new();
    // Samples of the frame being filled; the rest of a chunk carries into the next.
    let mut frame: Vec<i16> = Vec::with_capacity(FRAME_SAMPLES);
    loop {
        match chunks.recv().await {
            Ok(chunk) => {
                // An odd trailing byte cannot occur (chunks hold whole samples); it is ignored.
                let (samples, _odd_trailing_byte) = chunk.as_chunks::<2>();
                for bytes in samples {
                    frame.push(i16::from_le_bytes(*bytes));
                    if frame.len() < FRAME_SAMPLES {
                        continue;
                    }
                    // About 1 ms of CPU per second of audio on the Pi 5 (spike
                    // measurement), so this runs inline, not in `spawn_blocking`.
                    let score = detector.predict_i16(&frame);
                    frame.clear();
                    if state.push(score >= SPEECH_SCORE) {
                        return;
                    }
                }
            }
            Err(RecvError::Lagged(_)) => frame.clear(),
            // The recording is over; ending here would be a stop without speech then silence.
            Err(RecvError::Closed) => return pending().await,
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/features/listening/vad_tests.rs"]
mod tests;
