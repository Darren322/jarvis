//! The messages Jarvis and the screen UI (`ui/`) exchange over their WebSocket.
//!
//! One JSON object per message. Jarvis sends [`ToOrb`]; the UI sends [`FromOrb`].
//! This crate knows nothing about Jarvis or about drawing: each side maps its own
//! types to these.

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use thiserror::Error;

/// The longest line either side accepts, in bytes, not counting the newline.
///
/// The size of the widest message this crate encodes: a [`ToOrb::Stats`] with
/// `u64::MAX` for both RAM fields and both storage fields, both services down, and the widest `f32` text
/// for CPU and temperature. `serde_json` prints the shortest text that reads back
/// as the same `f32`: at most 9 significant digits, written without an exponent
/// from `1e-5` up to `1e16`. That makes the widest CPU text `0.00000123456789`
/// (16 bytes, CPU is 0 to 100) and the widest temperature `-1234567890000000.0`
/// (19 bytes). This is a bound by construction, not a value every f32 reaches
/// (sampled encodings stay below it). A test builds that line, checks it equals this
/// constant, and checks sampled encodings never exceed it. The limit also
/// bounds how much a reader buffers from a peer that never sends a newline.
pub const MAX_LINE_BYTES: usize = 274;

/// What Jarvis is doing, as the orb shows it. On the wire: `"idle"`,
/// `"listening"`, `"transcribing"`, `"thinking"`, `"speaking"`.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OrbState {
    Idle,
    Listening,
    Transcribing,
    Thinking,
    Speaking,
}

/// Whose loudness a level is. On the wire: `"speech"` (Jarvis's voice being
/// played) or `"mic"` (the microphone while listening).
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LevelSource {
    Speech,
    Mic,
}

/// A loudness from 0.0 to 1.0 inclusive, valid by construction.
///
/// A sender with a raw measurement uses [`Level::clamped`]. Decoding is strict:
/// a level outside the range (or a JSON `null`, which is how a NaN would arrive)
/// makes the whole message invalid, so the orb never draws from a bad value.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(try_from = "f32", into = "f32")]
pub struct Level(f32);

/// A level outside 0.0 to 1.0, or not a number.
#[derive(Clone, Copy, Debug, Error, PartialEq)]
#[error("level {0} is outside 0 to 1")]
pub struct LevelOutOfRange(pub f32);

impl Level {
    pub const SILENT: Self = Self(0.0);

    /// Accepts 0.0 to 1.0 inclusive and rejects everything else, including NaN.
    pub fn new(value: f32) -> Result<Self, LevelOutOfRange> {
        if (0.0..=1.0).contains(&value) {
            Ok(Self(value))
        } else {
            Err(LevelOutOfRange(value))
        }
    }

    /// Pulls a raw measurement into range: below 0 becomes 0, above 1 becomes 1,
    /// and NaN becomes 0.
    pub fn clamped(value: f32) -> Self {
        if value.is_nan() {
            Self::SILENT
        } else {
            Self(value.clamp(0.0, 1.0))
        }
    }

    pub fn value(self) -> f32 {
        self.0
    }
}

impl TryFrom<f32> for Level {
    type Error = LevelOutOfRange;

    fn try_from(value: f32) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<Level> for f32 {
    fn from(level: Level) -> Self {
        level.0
    }
}

/// A CPU load from 0.0 to 100.0 inclusive, valid by construction like [`Level`].
///
/// A sender with a raw measurement uses [`CpuPercent::clamped`]. Decoding is
/// strict: a value outside the range, or a JSON `null`, makes the message invalid.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(try_from = "f32", into = "f32")]
pub struct CpuPercent(f32);

/// A CPU percent outside 0 to 100, or not a number.
#[derive(Clone, Copy, Debug, Error, PartialEq)]
#[error("cpu percent {0} is outside 0 to 100")]
pub struct CpuPercentOutOfRange(pub f32);

impl CpuPercent {
    /// Accepts 0.0 to 100.0 inclusive and rejects everything else, including NaN.
    pub fn new(value: f32) -> Result<Self, CpuPercentOutOfRange> {
        if (0.0..=100.0).contains(&value) {
            Ok(Self(value))
        } else {
            Err(CpuPercentOutOfRange(value))
        }
    }

    /// Pulls a raw measurement into range; NaN becomes 0.
    pub fn clamped(value: f32) -> Self {
        if value.is_nan() {
            Self(0.0)
        } else {
            Self(value.clamp(0.0, 100.0))
        }
    }

    pub fn value(self) -> f32 {
        self.0
    }
}

impl TryFrom<f32> for CpuPercent {
    type Error = CpuPercentOutOfRange;

    fn try_from(value: f32) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<CpuPercent> for f32 {
    fn from(cpu: CpuPercent) -> Self {
        cpu.0
    }
}

/// A panel the orb can show. On the wire: `"stats"` or `"camera"`.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PanelName {
    Stats,
    /// The camera's live picture. Its frames do not travel in these messages:
    /// the UI fetches them from Jarvis's `/camera` route while this panel shows.
    Camera,
}

/// Whether a service answered its last health check. On the wire: `"up"` or `"down"`.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceHealth {
    Up,
    Down,
}

/// The space on one filesystem, in bytes. Both numbers come together or not at
/// all, so a message never carries half a reading. On the wire:
/// `{"available_bytes":207517036544,"total_bytes":251416899584}`, both keys
/// required, no others allowed.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StorageUsage {
    pub available_bytes: u64,
    pub total_bytes: u64,
}

/// Reads a `storage` value that is present: a JSON `null` is invalid, not "unknown".
/// Unknown is the key being absent.
fn present_storage<'de, D>(deserializer: D) -> Result<Option<StorageUsage>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    StorageUsage::deserialize(deserializer).map(Some)
}

/// Jarvis to the orb.
///
/// ```text
/// {"type":"state","state":"thinking"}
/// {"type":"level","level":0.25,"source":"mic"}
/// {"type":"show_panel","panel":"stats"}
/// {"type":"show_panel","panel":"camera"}
/// {"type":"hide_panel"}
/// {"type":"stats","cpu_percent":12.5,"temperature_c":55.0,"ram_used_bytes":2,"ram_total_bytes":8,"storage":{"available_bytes":207517036544,"total_bytes":251416899584},"llama":"up","bge":"down"}
/// ```
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ToOrb {
    /// Sent on every state change.
    State { state: OrbState },
    /// Sent about 30 times a second while there is something to hear.
    Level { level: Level, source: LevelSource },
    /// Opens a panel over the orb.
    ShowPanel { panel: PanelName },
    /// Closes whatever panel is open.
    HidePanel {},
    /// A reading of the Pi's health. A missing temperature sensor is an absent
    /// `temperature_c`, and an unreadable filesystem an absent `storage`, never 0.
    Stats {
        cpu_percent: CpuPercent,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        temperature_c: Option<f32>,
        ram_used_bytes: u64,
        ram_total_bytes: u64,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "present_storage"
        )]
        storage: Option<StorageUsage>,
        llama: ServiceHealth,
        bge: ServiceHealth,
    },
}

/// The orb to Jarvis.
///
/// ```text
/// {"type":"tap"}
/// ```
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum FromOrb {
    /// The screen was tapped. It carries nothing: Jarvis decides what a tap
    /// means from its own state.
    Tap {},
    /// Darren tapped outside the open panel, and the orb closed it.
    PanelClosed {},
}

#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("line is {len} bytes, over the limit of {MAX_LINE_BYTES}")]
    LineTooLong { len: usize },
    #[error("line is not a valid message")]
    Invalid(#[from] serde_json::Error),
}

impl ToOrb {
    /// The message as one line, ending in `\n`.
    pub fn encode(&self) -> Result<String, ProtocolError> {
        encode_line(self)
    }

    /// Reads one line, with or without its `\n`.
    pub fn decode(line: &str) -> Result<Self, ProtocolError> {
        decode_line(line)
    }
}

impl FromOrb {
    /// The message as one line, ending in `\n`.
    pub fn encode(&self) -> Result<String, ProtocolError> {
        encode_line(self)
    }

    /// Reads one line, with or without its `\n`.
    pub fn decode(line: &str) -> Result<Self, ProtocolError> {
        decode_line(line)
    }
}

fn encode_line<T: Serialize>(message: &T) -> Result<String, ProtocolError> {
    let mut line = serde_json::to_string(message)?;
    if line.len() > MAX_LINE_BYTES {
        return Err(ProtocolError::LineTooLong { len: line.len() });
    }
    line.push('\n');
    Ok(line)
}

fn decode_line<T: DeserializeOwned>(line: &str) -> Result<T, ProtocolError> {
    let line = line.strip_suffix('\n').unwrap_or(line);
    // Checked before parsing, so an oversized line costs no parse.
    if line.len() > MAX_LINE_BYTES {
        return Err(ProtocolError::LineTooLong { len: line.len() });
    }
    Ok(serde_json::from_str(line)?)
}
