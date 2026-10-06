use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use rodio::{
    Player,
    buffer::SamplesBuffer,
    cpal::{
        self,
        traits::{DeviceTrait, HostTrait},
    },
    stream::{DeviceSinkBuilder, DeviceSinkConfig, DeviceSinkError, MixerDeviceSink},
};

const OUTPUT_CHANNELS: u16 = 1;
const OUTPUT_SAMPLE_RATE: u32 = 44_100;
pub(crate) const WAKE_NOISE_DURATION: std::time::Duration = std::time::Duration::from_millis(700);
const WAKE_NOISE_SAMPLE_COUNT: usize = (OUTPUT_SAMPLE_RATE as usize * 7) / 10;
const WAKE_NOISE_PCM16_LIMIT: i16 = 75;

#[derive(Debug)]
pub enum PlaybackError {
    EnumerateDevices(cpal::DevicesError),
    OpenDevice(DeviceSinkError),
    RequestedDeviceNotFound(String),
    EmptySamples,
    DeviceFailed,
}

impl std::fmt::Display for PlaybackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EnumerateDevices(error) => {
                write!(f, "failed to list audio output devices: {error}")
            }
            Self::OpenDevice(error) => {
                write!(f, "failed to open audio output device: {error}")?;
                if let Some(source) = std::error::Error::source(error) {
                    write!(f, ": {source}")?;
                }
                Ok(())
            }
            Self::RequestedDeviceNotFound(name) => {
                write!(
                    f,
                    "configured audio output device identifier or name was not found: {name}"
                )
            }
            Self::EmptySamples => write!(f, "audio samples are empty"),
            Self::DeviceFailed => write!(f, "audio output device reported an error"),
        }
    }
}

impl std::error::Error for PlaybackError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::EnumerateDevices(error) => Some(error),
            Self::OpenDevice(error) => Some(error),
            _ => None,
        }
    }
}

pub struct AudioPlayer {
    device: MixerDeviceSink,
    player: Option<Player>,
    device_failed: Arc<AtomicBool>,
}

impl AudioPlayer {
    /// Opens one persistent output stream, using an exact configured device identifier or name.
    ///
    /// This performs blocking device discovery and must run on a blocking thread.
    pub fn new(requested_device: Option<String>) -> Result<Self, PlaybackError> {
        let host = cpal::default_host();
        let device = match requested_device {
            Some(requested_name) => host
                .output_devices()
                .map_err(PlaybackError::EnumerateDevices)?
                .find(|device| {
                    device.id().is_ok_and(|id| id.to_string() == requested_name)
                        || device
                            .description()
                            .is_ok_and(|description| description.name() == requested_name)
                })
                .ok_or(PlaybackError::RequestedDeviceNotFound(requested_name))?,
            None => host
                .default_output_device()
                .ok_or(PlaybackError::OpenDevice(DeviceSinkError::NoDevice))?,
        };

        let device_failed = Arc::new(AtomicBool::new(false));
        let callback_state = Arc::clone(&device_failed);
        let sink = DeviceSinkBuilder::from_device(device)
            .map_err(PlaybackError::OpenDevice)?
            .with_error_callback(move |_| {
                callback_state.store(true, Ordering::Release);
            })
            .open_sink_or_fallback()
            .map_err(PlaybackError::OpenDevice)?;

        Ok(Self {
            device: sink,
            player: None,
            device_failed,
        })
    }

    pub fn play(&mut self, samples: Vec<f32>) -> Result<(), PlaybackError> {
        if self.device_has_failed() {
            return Err(PlaybackError::DeviceFailed);
        }
        if samples.is_empty() {
            return Err(PlaybackError::EmptySamples);
        }

        self.stop();
        let samples = prepend_wake_noise(samples);

        let source = SamplesBuffer::new(
            OUTPUT_CHANNELS.try_into().expect("nonzero channel count"),
            OUTPUT_SAMPLE_RATE.try_into().expect("nonzero sample rate"),
            samples,
        );
        let player = Player::connect_new(self.device.mixer());
        player.append(source);
        self.player = Some(player);

        Ok(())
    }

    /// Stops and drops the per-utterance player while keeping the output stream open.
    pub fn stop(&mut self) {
        if let Some(player) = self.player.take() {
            player.stop();
        }
    }

    pub fn finish(&mut self) {
        self.player = None;
    }

    pub fn is_playing(&self) -> bool {
        self.player.as_ref().is_some_and(|player| !player.empty())
    }

    pub fn device_has_failed(&self) -> bool {
        self.device_failed.load(Ordering::Acquire)
    }

    pub fn output_config(&self) -> DeviceSinkConfig {
        *self.device.config()
    }
}

fn prepend_wake_noise(speech_samples: Vec<f32>) -> Vec<f32> {
    let mut samples = Vec::with_capacity(WAKE_NOISE_SAMPLE_COUNT + speech_samples.len());
    // A fixed non-cryptographic sequence is enough to produce unobtrusive audio noise.
    let mut state = 0x9E37_79B9_u32;

    for _ in 0..WAKE_NOISE_SAMPLE_COUNT {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        let pcm16 =
            (state % (WAKE_NOISE_PCM16_LIMIT as u32 * 2 + 1)) as i16 - WAKE_NOISE_PCM16_LIMIT;
        samples.push(f32::from(pcm16) / 32_768.0);
    }

    samples.extend(speech_samples);
    samples
}

#[cfg(test)]
#[path = "../../../tests/unit/services/tts/playback_tests.rs"]
mod tests;
