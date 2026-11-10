//! The messages Jarvis and `jarvis-orb` exchange over their Unix socket.
//!
//! One JSON object per line. Jarvis sends [`ToOrb`]; the orb sends [`FromOrb`].
//! This crate knows nothing about Jarvis or about drawing: each side maps its own
//! types to these.

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use thiserror::Error;

/// The longest line either side accepts, in bytes, not counting the newline.
///
/// The longest message this crate encodes is under 60 bytes, so the limit only
/// bounds how much a reader buffers from a peer that never sends a newline.
pub const MAX_LINE_BYTES: usize = 256;

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

/// Jarvis to the orb.
///
/// ```text
/// {"type":"state","state":"thinking"}
/// {"type":"level","level":0.25,"source":"mic"}
/// ```
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ToOrb {
    /// Sent on every state change.
    State { state: OrbState },
    /// Sent about 30 times a second while there is something to hear.
    Level { level: Level, source: LevelSource },
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
