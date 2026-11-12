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
    /// Colour of the sphere's points.
    pub node_color: Rgb,
    /// Colour of the links between points.
    pub link_color: Rgb,
    /// Colour of the glyph rings.
    pub ring_color: Rgb,
    /// Colour of the core glow and of the rays.
    pub core_color: Rgb,
    /// Overall brightness of the sphere, 0 (invisible) to 1.
    pub brightness: f32,
    /// Turn of the sphere, radians per second. The inner lattice turns the other way.
    pub sphere_spin: f32,
    /// Turn of the outer and inner glyph rings, radians per second. A negative
    /// value turns the other way.
    pub ring_spin: [f32; 2],
    /// How visible the glyph rings are, 0 to 1.
    pub ring_alpha: f32,
    /// How far the orb swells and shrinks as it breathes, as a fraction of its radius.
    pub breath_depth: f32,
    /// Breaths per second.
    pub breath_rate: f32,
    /// How strongly the points swell and brighten in a rippling pulse, 0 to 1.
    pub node_pulse: f32,
    /// How far the points move in and out of the sphere, 0 to 1.
    pub node_jitter: f32,
    /// Size and brightness of the core glow, 0 to 1.
    pub core_glow: f32,
    /// Length and brightness of the light rays, 0 (none) to 1.
    pub ray_strength: f32,
}

// Per-state palettes. Offline is neutral grey; Thinking's core is gold; the
// other cores are warm and softer.
const OFFLINE_NODE: Rgb = [0.60, 0.60, 0.60];
const OFFLINE_LINK: Rgb = [0.45, 0.45, 0.45];
const OFFLINE_RING: Rgb = [0.50, 0.50, 0.50];
const OFFLINE_CORE: Rgb = [0.55, 0.55, 0.55];
const IDLE_NODE: Rgb = [0.45, 0.72, 1.00];
const IDLE_LINK: Rgb = [0.20, 0.50, 0.95];
const IDLE_RING: Rgb = [0.35, 0.62, 1.00];
const IDLE_CORE: Rgb = [1.00, 0.62, 0.30];
const LISTENING_NODE: Rgb = [0.50, 0.90, 1.00];
const LISTENING_LINK: Rgb = [0.20, 0.70, 1.00];
const LISTENING_RING: Rgb = [0.40, 0.80, 1.00];
const LISTENING_CORE: Rgb = [1.00, 0.72, 0.40];
const TRANSCRIBING_NODE: Rgb = [0.62, 0.72, 1.00];
const TRANSCRIBING_LINK: Rgb = [0.38, 0.50, 1.00];
const TRANSCRIBING_RING: Rgb = [0.75, 0.82, 1.00];
const TRANSCRIBING_CORE: Rgb = [1.00, 0.70, 0.38];
const THINKING_NODE: Rgb = [0.50, 0.65, 1.00];
const THINKING_LINK: Rgb = [0.30, 0.40, 0.95];
const THINKING_RING: Rgb = [0.60, 0.75, 1.00];
const THINKING_CORE: Rgb = [1.00, 0.78, 0.22];
const SPEAKING_NODE: Rgb = [0.70, 0.95, 1.00];
const SPEAKING_LINK: Rgb = [0.30, 0.80, 1.00];
const SPEAKING_RING: Rgb = [0.50, 0.85, 1.00];
const SPEAKING_CORE: Rgb = [1.00, 0.85, 0.55];

/// The look for `view` at a raw `level`.
///
/// Only Listening (the microphone) and Speaking (Jarvis's voice) use the level;
/// the other views ignore it. The level is clamped to 0..1 (NaN counts as 0) and
/// passed through [`level_response`].
pub fn look(view: View, level: f32) -> Look {
    let response = level_response(level);
    match view {
        View::Offline => Look {
            node_color: OFFLINE_NODE,
            link_color: OFFLINE_LINK,
            ring_color: OFFLINE_RING,
            core_color: OFFLINE_CORE,
            brightness: 0.30,
            sphere_spin: 0.06,
            ring_spin: [0.04, -0.03],
            ring_alpha: 0.25,
            breath_depth: 0.035,
            breath_rate: 0.20,
            node_pulse: 0.0,
            node_jitter: 0.0,
            core_glow: 0.18,
            ray_strength: 0.0,
        },
        View::State(OrbState::Idle) => Look {
            node_color: IDLE_NODE,
            link_color: IDLE_LINK,
            ring_color: IDLE_RING,
            core_color: IDLE_CORE,
            brightness: 0.60,
            sphere_spin: 0.14,
            ring_spin: [0.10, -0.07],
            ring_alpha: 0.45,
            breath_depth: 0.045,
            breath_rate: 0.25,
            node_pulse: 0.0,
            node_jitter: 0.0,
            core_glow: 0.35,
            ray_strength: 0.10,
        },
        View::State(OrbState::Listening) => Look {
            node_color: LISTENING_NODE,
            link_color: LISTENING_LINK,
            ring_color: LISTENING_RING,
            core_color: LISTENING_CORE,
            brightness: 0.70 + 0.30 * response,
            sphere_spin: 0.18,
            ring_spin: [0.12, -0.08],
            ring_alpha: 0.50,
            breath_depth: 0.030,
            breath_rate: 0.35,
            node_pulse: response,
            node_jitter: 0.0,
            core_glow: 0.40 + 0.40 * response,
            ray_strength: 0.10,
        },
        View::State(OrbState::Transcribing) => Look {
            node_color: TRANSCRIBING_NODE,
            link_color: TRANSCRIBING_LINK,
            ring_color: TRANSCRIBING_RING,
            core_color: TRANSCRIBING_CORE,
            brightness: 0.75,
            sphere_spin: 0.16,
            // The outer ring spins fast: the "working on your words" cue.
            ring_spin: [2.6, -0.35],
            ring_alpha: 0.90,
            breath_depth: 0.030,
            breath_rate: 0.50,
            node_pulse: 0.0,
            node_jitter: 0.0,
            core_glow: 0.45,
            ray_strength: 0.15,
        },
        View::State(OrbState::Thinking) => Look {
            node_color: THINKING_NODE,
            link_color: THINKING_LINK,
            ring_color: THINKING_RING,
            core_color: THINKING_CORE,
            brightness: 0.80,
            sphere_spin: 0.22,
            // Opposite directions: the rings grind against each other.
            ring_spin: [1.3, -1.7],
            ring_alpha: 0.85,
            breath_depth: 0.050,
            breath_rate: 0.80,
            node_pulse: 0.0,
            node_jitter: 0.0,
            core_glow: 0.95,
            ray_strength: 0.30,
        },
        View::State(OrbState::Speaking) => Look {
            node_color: SPEAKING_NODE,
            link_color: SPEAKING_LINK,
            ring_color: SPEAKING_RING,
            core_color: SPEAKING_CORE,
            brightness: 0.75 + 0.25 * response,
            sphere_spin: 0.20,
            ring_spin: [0.20, -0.15],
            ring_alpha: 0.60,
            breath_depth: 0.040,
            breath_rate: 0.50,
            node_pulse: 0.35 * response,
            node_jitter: response,
            core_glow: 0.50 + 0.50 * response,
            ray_strength: 0.15 + 0.85 * response,
        },
    }
}

fn mix(from: f32, to: f32, t: f32) -> f32 {
    from + (to - from) * t
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
            node_color: mix_rgb(self.node_color, other.node_color, t),
            link_color: mix_rgb(self.link_color, other.link_color, t),
            ring_color: mix_rgb(self.ring_color, other.ring_color, t),
            core_color: mix_rgb(self.core_color, other.core_color, t),
            brightness: mix(self.brightness, other.brightness, t),
            sphere_spin: mix(self.sphere_spin, other.sphere_spin, t),
            ring_spin: [
                mix(self.ring_spin[0], other.ring_spin[0], t),
                mix(self.ring_spin[1], other.ring_spin[1], t),
            ],
            ring_alpha: mix(self.ring_alpha, other.ring_alpha, t),
            breath_depth: mix(self.breath_depth, other.breath_depth, t),
            breath_rate: mix(self.breath_rate, other.breath_rate, t),
            node_pulse: mix(self.node_pulse, other.node_pulse, t),
            node_jitter: mix(self.node_jitter, other.node_jitter, t),
            core_glow: mix(self.core_glow, other.core_glow, t),
            ray_strength: mix(self.ray_strength, other.ray_strength, t),
        }
    }
}
