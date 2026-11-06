//! The sample rates a speech worker may declare and the pipeline will play.

use std::num::NonZeroU32;

/// Lowest accepted rate: below this, speech is too narrow to understand.
pub(super) const MIN_SAMPLE_RATE_HZ: u32 = 16_000;
/// Highest accepted rate. It also bounds the audio buffers (see [`SampleRate::max_samples`]).
pub(super) const MAX_SAMPLE_RATE_HZ: u32 = 48_000;
/// Longest utterance a worker may return.
const MAX_AUDIO_SECONDS: u32 = 120;

/// A worker's sample rate, known to be within [`MIN_SAMPLE_RATE_HZ`]..=[`MAX_SAMPLE_RATE_HZ`].
///
/// Piper declares 22,050 Hz and Supertonic 44,100 Hz. The player resamples to the output
/// device, so Jarvis plays each rate as declared and never converts it itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct SampleRate(rodio::SampleRate);

impl SampleRate {
    /// Returns `None` for a rate outside the accepted range.
    pub fn new(hz: u32) -> Option<Self> {
        if !(MIN_SAMPLE_RATE_HZ..=MAX_SAMPLE_RATE_HZ).contains(&hz) {
            return None;
        }
        NonZeroU32::new(hz).map(Self)
    }

    pub fn hz(self) -> u32 {
        self.0.get()
    }

    pub fn for_rodio(self) -> rodio::SampleRate {
        self.0
    }

    /// The most samples one utterance may hold: [`MAX_AUDIO_SECONDS`] of audio at this rate.
    pub fn max_samples(self) -> usize {
        // A u32 holds at most 120 * 48,000 here, and usize is at least 32 bits.
        usize::try_from(self.hz() * MAX_AUDIO_SECONDS).unwrap_or(usize::MAX)
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/features/speech/sample_rate_tests.rs"]
mod tests;
