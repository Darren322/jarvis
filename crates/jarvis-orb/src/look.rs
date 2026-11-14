//! What the orb should look like for a given view and loudness.
//!
//! Plain data and pure functions only: no drawing types and no clock. The drawing
//! code in the binary turns a [`Look`] into pixels, and eases from one `Look` to
//! the next with [`Look::lerp`] so a change of state never snaps.

use orb_protocol::{Level, OrbState};

/// A colour as red, green, blue, each 0.0 to 1.0.
pub type Rgb = [f32; 3];

/// What the orb is showing: one of Jarvis's states, or the offline look shown
/// while the orb is not connected to Jarvis.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum View {
    Offline,
    State(OrbState),
}

impl View {
    /// Every view, in the order [`View::next`] cycles through them.
    pub const ALL: [Self; 6] = [
        Self::Offline,
        Self::State(OrbState::Idle),
        Self::State(OrbState::Listening),
        Self::State(OrbState::Transcribing),
        Self::State(OrbState::Thinking),
        Self::State(OrbState::Speaking),
    ];

    /// The view after this one, wrapping from Speaking back to Offline.
    pub fn next(self) -> Self {
        match self {
            Self::Offline => Self::State(OrbState::Idle),
            Self::State(OrbState::Idle) => Self::State(OrbState::Listening),
            Self::State(OrbState::Listening) => Self::State(OrbState::Transcribing),
            Self::State(OrbState::Transcribing) => Self::State(OrbState::Thinking),
            Self::State(OrbState::Thinking) => Self::State(OrbState::Speaking),
            Self::State(OrbState::Speaking) => Self::Offline,
        }
    }
}

/// How sharply the level curve rises: the response is `1 - exp(-LEVEL_GAIN * level)`.
///
/// Speech RMS is small: roughly 0.01 to 0.1 from the microphone and below that
/// from playback. At a gain of 18, a level of 0.01 gives a response of about
/// 0.16, 0.05 gives about 0.59, 0.1 gives about 0.83 and 0.3 is nearly full, so
/// normal speech moves the orb visibly and loud speech does not saturate it
/// until it is genuinely loud.
pub const LEVEL_GAIN: f32 = 18.0;

/// How fast one look eases into the next, per second. The remaining distance
/// shrinks by a factor of `exp(-EASE_RATE * dt)`, so about 95% of a change is
/// done after `3 / EASE_RATE`, a third of a second.
pub const EASE_RATE: f32 = 9.0;

/// Maps a raw 0..1 level to a lively 0..1 response.
///
/// Out-of-range and NaN input is clamped first. The curve is 0 at 0, strictly
/// increasing, and reaches 1.0 (in `f32`) at a level of 1.0.
pub fn level_response(level: f32) -> f32 {
    let level = Level::clamped(level).value();
    1.0 - (-LEVEL_GAIN * level).exp()
}

/// The fraction of the way to the target to move this frame after `dt` seconds.
/// A negative or NaN `dt` moves nothing.
pub fn ease_factor(dt: f32) -> f32 {
    if dt.is_nan() || dt <= 0.0 {
        0.0
    } else {
        1.0 - (-EASE_RATE * dt).exp()
    }
}

/// Everything the drawing needs to know. Every field is a number or a colour so
/// two looks can be blended field by field.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    /// Colour of the starburst, the core ring and the rays.
    pub core_color: Rgb,
    /// Colour of the translucent fill inside the core ring and behind the glyph band.
    pub fill_color: Rgb,
    /// Colour of the band of tall glyphs around the core.
    pub band_color: Rgb,
    /// Colour of the HUD rings and ticks: the cool, technical frame.
    pub hud_color: Rgb,
    /// Overall brightness of the mandala, 0 (invisible) to 1.
    pub brightness: f32,
    /// Turn of the sparkle cloud inside the core ring, radians per second.
    pub sparkle_spin: f32,
    /// Turn of the glyph band and of the HUD rings, radians per second. A negative
    /// value turns the other way. The HUD rings turn at fixed multiples of the
    /// second value.
    pub band_spin: [f32; 2],
    /// How visible the glyph band is, 0 to 1.
    pub band_alpha: f32,
    /// How visible the HUD rings and ticks are, 0 to 1.
    pub hud_alpha: f32,
    /// How far the orb swells and shrinks as it breathes, as a fraction of its radius.
    pub breath_depth: f32,
    /// Breaths per second.
    pub breath_rate: f32,
    /// How strongly the core ring and the sparkles swell and brighten in a
    /// rippling pulse, 0 to 1.
    pub node_pulse: f32,
    /// How far the sparkles move in and out of the core ring, 0 to 1. The
    /// outermost HUD ring also flares with it.
    pub node_jitter: f32,
    /// Size and brightness of the core, 0 to 1.
    pub core_glow: f32,
    /// Length and brightness of the light rays, 0 (none) to 1.
    pub ray_strength: f32,
    /// How visible the drifting background blobs of light are, 0 to 1.
    pub background: f32,
    /// How coloured the background blobs are: 0 is grey, 1 is full colour.
    pub blob_saturation: f32,
}

// Per-state palettes. Offline is neutral grey; Idle is a cool blue HUD with
// a soft amber core; Thinking is the full gold look.
const OFFLINE_CORE: Rgb = [0.55, 0.55, 0.55];
const OFFLINE_FILL: Rgb = [0.40, 0.40, 0.40];
const OFFLINE_BAND: Rgb = [0.50, 0.50, 0.50];
const OFFLINE_HUD: Rgb = [0.45, 0.45, 0.45];
const IDLE_CORE: Rgb = [1.00, 0.62, 0.30];
const IDLE_FILL: Rgb = [0.85, 0.45, 0.10];
const IDLE_BAND: Rgb = [0.80, 0.52, 0.22];
const IDLE_HUD: Rgb = [0.15, 0.55, 0.95];
const LISTENING_CORE: Rgb = [1.00, 0.72, 0.40];
const LISTENING_FILL: Rgb = [0.90, 0.55, 0.15];
const LISTENING_BAND: Rgb = [0.95, 0.70, 0.30];
const LISTENING_HUD: Rgb = [0.25, 0.85, 1.00];
const TRANSCRIBING_CORE: Rgb = [1.00, 0.70, 0.38];
const TRANSCRIBING_FILL: Rgb = [0.95, 0.50, 0.10];
const TRANSCRIBING_BAND: Rgb = [1.00, 0.68, 0.22];
const TRANSCRIBING_HUD: Rgb = [0.20, 0.65, 1.00];
const THINKING_CORE: Rgb = [1.00, 0.78, 0.22];
const THINKING_FILL: Rgb = [1.00, 0.55, 0.08];
const THINKING_BAND: Rgb = [1.00, 0.60, 0.15];
const THINKING_HUD: Rgb = [0.25, 0.90, 1.00];
const SPEAKING_CORE: Rgb = [1.00, 0.85, 0.55];
const SPEAKING_FILL: Rgb = [1.00, 0.60, 0.15];
const SPEAKING_BAND: Rgb = [0.95, 0.72, 0.30];
const SPEAKING_HUD: Rgb = [0.25, 0.80, 1.00];

/// The look for `view` at a raw `level`.
///
/// Only Listening (the microphone) and Speaking (Jarvis's voice) use the level;
/// the other views ignore it. The level is clamped to 0..1 (NaN counts as 0) and
/// passed through [`level_response`].
pub fn look(view: View, level: f32) -> Look {
    let response = level_response(level);
    match view {
        View::Offline => Look {
            core_color: OFFLINE_CORE,
            fill_color: OFFLINE_FILL,
            band_color: OFFLINE_BAND,
            hud_color: OFFLINE_HUD,
            brightness: 0.30,
            sparkle_spin: 0.06,
            band_spin: [0.04, -0.03],
            band_alpha: 0.25,
            hud_alpha: 0.30,
            breath_depth: 0.035,
            breath_rate: 0.20,
            node_pulse: 0.0,
            node_jitter: 0.0,
            core_glow: 0.18,
            ray_strength: 0.0,
            background: 0.30,
            blob_saturation: 0.0,
        },
        View::State(OrbState::Idle) => Look {
            core_color: IDLE_CORE,
            fill_color: IDLE_FILL,
            band_color: IDLE_BAND,
            hud_color: IDLE_HUD,
            brightness: 0.60,
            sparkle_spin: 0.14,
            band_spin: [0.10, -0.07],
            band_alpha: 0.35,
            hud_alpha: 0.85,
            breath_depth: 0.045,
            breath_rate: 0.25,
            node_pulse: 0.0,
            node_jitter: 0.0,
            core_glow: 0.35,
            ray_strength: 0.10,
            background: 0.50,
            blob_saturation: 0.60,
        },
        View::State(OrbState::Listening) => Look {
            core_color: LISTENING_CORE,
            fill_color: LISTENING_FILL,
            band_color: LISTENING_BAND,
            hud_color: LISTENING_HUD,
            brightness: 0.70 + 0.30 * response,
            sparkle_spin: 0.18,
            band_spin: [0.12, -0.08],
            band_alpha: 0.55,
            hud_alpha: 0.70 + 0.30 * response,
            breath_depth: 0.030,
            breath_rate: 0.35,
            node_pulse: response,
            node_jitter: 0.0,
            core_glow: 0.40 + 0.40 * response,
            ray_strength: 0.10,
            background: 0.60,
            blob_saturation: 0.80,
        },
        View::State(OrbState::Transcribing) => Look {
            core_color: TRANSCRIBING_CORE,
            fill_color: TRANSCRIBING_FILL,
            band_color: TRANSCRIBING_BAND,
            hud_color: TRANSCRIBING_HUD,
            brightness: 0.75,
            sparkle_spin: 0.16,
            // The glyph band spins fast: the "working on your words" cue.
            band_spin: [2.4, -0.30],
            band_alpha: 0.90,
            hud_alpha: 0.60,
            breath_depth: 0.030,
            breath_rate: 0.50,
            node_pulse: 0.0,
            node_jitter: 0.0,
            core_glow: 0.45,
            ray_strength: 0.15,
            background: 0.60,
            blob_saturation: 0.90,
        },
        View::State(OrbState::Thinking) => Look {
            core_color: THINKING_CORE,
            fill_color: THINKING_FILL,
            band_color: THINKING_BAND,
            hud_color: THINKING_HUD,
            brightness: 0.85,
            sparkle_spin: 0.22,
            // Opposite directions: the glyph band and the HUD rings grind against each other.
            band_spin: [1.0, -0.80],
            band_alpha: 0.90,
            hud_alpha: 0.95,
            breath_depth: 0.050,
            breath_rate: 0.80,
            node_pulse: 0.0,
            node_jitter: 0.0,
            core_glow: 0.95,
            ray_strength: 0.55,
            background: 0.80,
            blob_saturation: 1.0,
        },
        View::State(OrbState::Speaking) => Look {
            core_color: SPEAKING_CORE,
            fill_color: SPEAKING_FILL,
            band_color: SPEAKING_BAND,
            hud_color: SPEAKING_HUD,
            brightness: 0.75 + 0.25 * response,
            sparkle_spin: 0.20,
            band_spin: [0.20, -0.15],
            band_alpha: 0.65,
            hud_alpha: 0.65,
            breath_depth: 0.040,
            breath_rate: 0.50,
            node_pulse: 0.35 * response,
            node_jitter: response,
            core_glow: 0.50 + 0.50 * response,
            ray_strength: 0.15 + 0.85 * response,
            background: 0.70,
            blob_saturation: 1.0,
        },
    }
}

/// Exact at both ends: `t` of 0.0 gives `from` and 1.0 gives `to`, with no
/// rounding left over, so a finished transition equals its target.
fn mix(from: f32, to: f32, t: f32) -> f32 {
    from * (1.0 - t) + to * t
}

fn mix_rgb(from: Rgb, to: Rgb, t: f32) -> Rgb {
    [
        mix(from[0], to[0], t),
        mix(from[1], to[1], t),
        mix(from[2], to[2], t),
    ]
}

impl Look {
    /// Blends this look towards `other`: `t` of 0.0 gives this look and 1.0 gives
    /// `other`. `t` is clamped to 0..1 (NaN counts as 0).
    pub fn lerp(&self, other: &Self, t: f32) -> Self {
        let t = Level::clamped(t).value();
        Self {
            core_color: mix_rgb(self.core_color, other.core_color, t),
            fill_color: mix_rgb(self.fill_color, other.fill_color, t),
            band_color: mix_rgb(self.band_color, other.band_color, t),
            hud_color: mix_rgb(self.hud_color, other.hud_color, t),
            brightness: mix(self.brightness, other.brightness, t),
            sparkle_spin: mix(self.sparkle_spin, other.sparkle_spin, t),
            band_spin: [
                mix(self.band_spin[0], other.band_spin[0], t),
                mix(self.band_spin[1], other.band_spin[1], t),
            ],
            band_alpha: mix(self.band_alpha, other.band_alpha, t),
            hud_alpha: mix(self.hud_alpha, other.hud_alpha, t),
            breath_depth: mix(self.breath_depth, other.breath_depth, t),
            breath_rate: mix(self.breath_rate, other.breath_rate, t),
            node_pulse: mix(self.node_pulse, other.node_pulse, t),
            node_jitter: mix(self.node_jitter, other.node_jitter, t),
            core_glow: mix(self.core_glow, other.core_glow, t),
            ray_strength: mix(self.ray_strength, other.ray_strength, t),
            background: mix(self.background, other.background, t),
            blob_saturation: mix(self.blob_saturation, other.blob_saturation, t),
        }
    }
}
