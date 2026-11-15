//! Short sounds that mark what Jarvis is doing, so it can be followed without
//! looking at the screen.
//!
//! Each cue is one WAV file in a configured folder. The files are loaded once at
//! startup and played through the audio output that speech already keeps open, so
//! a cue and the voice share one device and never compete for it.

use std::{convert::Infallible, path::Path, sync::Arc, time::Duration};

use rodio::{buffer::SamplesBuffer, mixer::Mixer};
use thiserror::Error;
use tokio::sync::watch;

use super::sample_rate::SampleRate;
use crate::features::state::{AssistantState, StateObserver};

/// The longest cue accepted. A cue marks a moment; anything longer would run into
/// the voice or the next cue.
const MAX_CUE_MILLIS: u32 = 2_000;
const EXPECTED_CHANNELS: u16 = 1;
const EXPECTED_BITS_PER_SAMPLE: u16 = 16;
const CUE_COUNT: usize = 6;

/// A moment that has its own sound.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Cue {
    /// Played before the microphone opens: "speak now".
    Listening,
    Transcribing,
    Thinking,
    Speaking,
    /// A turn that ended after Jarvis spoke.
    Idle,
    /// A turn that ended with nothing spoken.
    Error,
}

impl Cue {
    pub(crate) const ALL: [Cue; CUE_COUNT] = [
        Cue::Listening,
        Cue::Transcribing,
        Cue::Thinking,
        Cue::Speaking,
        Cue::Idle,
        Cue::Error,
    ];

    /// The file this cue is loaded from, inside the cue folder.
    pub(crate) fn file_name(self) -> &'static str {
        match self {
            Cue::Listening => "listening.wav",
            Cue::Transcribing => "transcribing.wav",
            Cue::Thinking => "thinking.wav",
            Cue::Speaking => "speaking.wav",
            Cue::Idle => "idle.wav",
            Cue::Error => "error.wav",
        }
    }

    fn index(self) -> usize {
        match self {
            Cue::Listening => 0,
            Cue::Transcribing => 1,
            Cue::Thinking => 2,
            Cue::Speaking => 3,
            Cue::Idle => 4,
            Cue::Error => 5,
        }
    }
}

#[derive(Debug, Error)]
pub(crate) enum CueError {
    #[error("the sound file could not be opened")]
    Open(#[source] hound::Error),
    #[error("the sound file must be 16-bit mono PCM at 16,000 to 48,000 Hz")]
    Format,
    #[error("the sound file holds no audio")]
    Empty,
    #[error("the sound file is longer than 2 seconds")]
    TooLong,
    #[error("the sound file could not be read")]
    Decode(#[source] hound::Error),
}

/// A cue that could not be loaded, and so stays silent.
#[derive(Debug)]
pub(crate) struct CueLoadFailure {
    pub(crate) cue: Cue,
    pub(crate) error: CueError,
}

struct CueSound {
    samples: Vec<f32>,
    rate: SampleRate,
}

impl CueSound {
    fn duration(&self) -> Duration {
        Duration::from_secs_f64(self.samples.len() as f64 / f64::from(self.rate.hz()))
    }
}

/// The cues that loaded. A cue without a sound is silent; nothing else changes.
#[derive(Default)]
pub(crate) struct CueSounds {
    sounds: [Option<CueSound>; CUE_COUNT],
}

impl CueSounds {
    /// Reads every cue file in `dir`, scaled by `volume` (above 0, up to 1). A file
    /// that is missing or unusable leaves its cue silent and is reported back.
    ///
    /// This reads files and must run on a blocking thread.
    pub(crate) fn load(dir: &Path, volume: f32) -> (Self, Vec<CueLoadFailure>) {
        let mut sounds = Self::default();
        let mut failures = Vec::new();
        for cue in Cue::ALL {
            match decode(&dir.join(cue.file_name()), volume) {
                Ok(sound) => sounds.sounds[cue.index()] = Some(sound),
                Err(error) => failures.push(CueLoadFailure { cue, error }),
            }
        }
        (sounds, failures)
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.sounds.iter().all(Option::is_none)
    }
}

fn decode(path: &Path, volume: f32) -> Result<CueSound, CueError> {
    let mut reader = hound::WavReader::open(path).map_err(CueError::Open)?;
    let spec = reader.spec();
    let rate = SampleRate::new(spec.sample_rate).ok_or(CueError::Format)?;
    if spec.channels != EXPECTED_CHANNELS
        || spec.bits_per_sample != EXPECTED_BITS_PER_SAMPLE
        || spec.sample_format != hound::SampleFormat::Int
    {
        return Err(CueError::Format);
    }

    // At most 48,000 * 2,000 / 1,000 = 96,000, so neither step overflows a u32.
    let max_samples =
        usize::try_from(rate.hz() * MAX_CUE_MILLIS / 1_000).expect("usize is at least 32 bits");
    let mut samples = Vec::new();
    for sample in reader.samples::<i16>() {
        if samples.len() == max_samples {
            return Err(CueError::TooLong);
        }
        let sample = sample.map_err(CueError::Decode)?;
        samples.push(f32::from(sample) / 32_768.0 * volume);
    }
    if samples.is_empty() {
        return Err(CueError::Empty);
    }
    Ok(CueSound { samples, rate })
}

/// Plays cues through whichever audio output is open right now.
///
/// Cheap to clone: every clone shares the loaded sounds and follows the same
/// output. With no output open, or no sound for a cue, playing does nothing.
#[derive(Clone)]
pub(crate) struct CuePlayer {
    sounds: Arc<CueSounds>,
    mixer: watch::Receiver<Option<Mixer>>,
}

impl CuePlayer {
    pub(super) fn new(sounds: Arc<CueSounds>, mixer: watch::Receiver<Option<Mixer>>) -> Self {
        Self { sounds, mixer }
    }

    /// Starts `cue` and returns how long it plays, without waiting for it. Returns
    /// zero when nothing was played. A cue mixes with whatever else is playing.
    pub(crate) fn play(&self, cue: Cue) -> Duration {
        let Some(sound) = &self.sounds.sounds[cue.index()] else {
            return Duration::ZERO;
        };
        let mixer = self.mixer.borrow();
        let Some(mixer) = mixer.as_ref() else {
            return Duration::ZERO;
        };
        mixer.add(SamplesBuffer::new(
            EXPECTED_CHANNELS.try_into().expect("nonzero channel count"),
            sound.rate.for_rodio(),
            sound.samples.clone(),
        ));
        sound.duration()
    }
}

/// Decides which cue a state change plays.
///
/// `Listening` plays nothing here: its cue is played before the microphone opens,
/// so that the recording never contains it. Reaching `Idle` plays the idle cue
/// after Jarvis spoke in that turn, and the error cue otherwise: nothing was
/// heard, speech-to-text or the model failed, the turn was cancelled, or the
/// answer was only printed.
#[derive(Default)]
pub(crate) struct CueSequence {
    spoke_this_turn: bool,
}

impl CueSequence {
    pub(crate) fn on_state(&mut self, state: AssistantState) -> Option<Cue> {
        match state {
            AssistantState::Listening => {
                // A new turn starts, whatever the last one did.
                self.spoke_this_turn = false;
                None
            }
            AssistantState::Transcribing => Some(Cue::Transcribing),
            AssistantState::Thinking => Some(Cue::Thinking),
            AssistantState::Speaking => {
                self.spoke_this_turn = true;
                Some(Cue::Speaking)
            }
            AssistantState::Idle => {
                if std::mem::take(&mut self.spoke_this_turn) {
                    Some(Cue::Idle)
                } else {
                    Some(Cue::Error)
                }
            }
        }
    }
}

/// Plays the cue for every state change, for as long as it is polled. Never
/// returns. Safe to drop at any `.await`: a cue that has started plays out.
pub(crate) async fn run_state_cues(mut states: StateObserver, player: CuePlayer) -> Infallible {
    let mut sequence = CueSequence::default();
    loop {
        let state = states.next().await;
        if let Some(cue) = sequence.on_state(state) {
            player.play(cue);
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/features/speech/cues_tests.rs"]
mod tests;
