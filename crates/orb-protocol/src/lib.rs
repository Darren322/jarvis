//! The messages Jarvis and the screen UI (`ui/`) exchange over their WebSocket.
//!
//! One JSON object per message. Jarvis sends [`ToOrb`]; the UI sends [`FromOrb`].
//! This crate knows nothing about Jarvis or about drawing: each side maps its own
//! types to these.

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use thiserror::Error;

/// The longest line Jarvis reads from the orb (a [`FromOrb`]), in bytes, not
/// counting the newline. It is also the WebSocket's read limit.
///
/// Every `FromOrb` is far shorter. The value is the size of the widest
/// [`ToOrb::Stats`], which was the widest message before built Panels, kept so
/// the transport's read limit did not change: `u64::MAX` for both RAM fields and
/// both storage fields, both services down, and the widest `f32` text for CPU
/// and temperature. `serde_json` prints the shortest text that reads back as the
/// same `f32`: at most 9 significant digits, written without an exponent from
/// `1e-5` up to `1e16`. That makes the widest CPU text `0.00000123456789` (16
/// bytes, CPU is 0 to 100) and the widest temperature `-1234567890000000.0` (19
/// bytes). A test builds that line and checks it equals this constant. The limit
/// also bounds how much a reader buffers from a peer that never sends a newline.
pub const MAX_LINE_BYTES: usize = 274;

/// The most characters in a built Panel's title (jarvis-ui 12, G4).
pub const MAX_PANEL_TITLE_CHARS: usize = 40;
/// The fewest parts a built Panel has.
pub const MIN_PANEL_PARTS: usize = 1;
/// The most parts a built Panel has (G4; P5's 8 became 4).
pub const MAX_PANEL_PARTS: usize = 4;
/// The fewest items in a list part.
pub const MIN_LIST_ITEMS: usize = 1;
/// The most items in a list part.
pub const MAX_LIST_ITEMS: usize = 8;
/// The most characters in a card's title or text, a list's title, or one list item.
pub const MAX_PART_TEXT_CHARS: usize = 120;

/// The most bytes `serde_json` writes for one character inside a string: a
/// control character such as U+0001 becomes `\u0001`. Any other character takes
/// at most 4 (UTF-8 is never escaped).
const MAX_ESCAPED_CHAR_BYTES: usize = 6;

/// The bytes of a JSON string of at most `chars` characters, quotes included.
const fn widest_string(chars: usize) -> usize {
    2 + chars * MAX_ESCAPED_CHAR_BYTES
}

/// The widest part: a list of the most items, every text at its limit. A card
/// holds two texts and a gauge none, so a list is always wider.
const WIDEST_PART_BYTES: usize = r#"{"kind":"list","title":"#.len()
    + widest_string(MAX_PART_TEXT_CHARS)
    + r#","items":["#.len()
    + MAX_LIST_ITEMS * widest_string(MAX_PART_TEXT_CHARS)
    + (MAX_LIST_ITEMS - 1)
    + "]}".len();

/// The most characters in a timer's label (timers 05). Labels are short spoken
/// names ("Time to sleep" is 13); 60 fits a strip row on the screen and keeps the
/// widest `timers` line (5 timers, every character escaped to six bytes, about
/// 2.3 KB, 2,283 bytes) far under [`MAX_TO_ORB_LINE_BYTES`].
pub const MAX_TIMER_LABEL_CHARS: usize = 60;
/// The most timers a `timers` message lists (timers D3, pending ones included).
pub const MAX_TIMERS: usize = 5;

/// The widest timer: the largest id and `ends_at_ms` (`u64::MAX`, 20 digits), a
/// label of the most characters at six bytes each, and the longest state
/// (`"pending"` and `"running"`, 9 bytes with quotes).
const WIDEST_TIMER_BYTES: usize = r#"{"id":"#.len()
    + u64::MAX.ilog10() as usize
    + 1
    + r#","label":"#.len()
    + widest_string(MAX_TIMER_LABEL_CHARS)
    + r#","ends_at_ms":"#.len()
    + u64::MAX.ilog10() as usize
    + 1
    + r#","state":"pending"}"#.len();

/// The widest [`ToOrb::Timers`]: the most timers, each the widest timer.
const WIDEST_TIMERS_BYTES: usize = r#"{"type":"timers","timers":["#.len()
    + MAX_TIMERS * WIDEST_TIMER_BYTES
    + (MAX_TIMERS - 1)
    + "]}".len();

/// The bytes of the widest [`ToOrb::ShowBuiltPanel`] the limits allow: the most
/// parts, each the widest part, and every character escaped to six bytes.
const WIDEST_BUILT_PANEL_LINE_BYTES: usize = r#"{"type":"show_built_panel","title":"#.len()
    + widest_string(MAX_PANEL_TITLE_CHARS)
    + r#","parts":["#.len()
    + MAX_PANEL_PARTS * WIDEST_PART_BYTES
    + (MAX_PANEL_PARTS - 1)
    + "]}".len();

/// The longest line Jarvis sends the orb (a [`ToOrb`]), in bytes, not counting
/// the newline: the wider of the widest [`ToOrb::ShowBuiltPanel`] and the widest
/// [`ToOrb::Timers`] the limits allow. A bound by construction; tests build both
/// lines and check the larger equals this.
pub const MAX_TO_ORB_LINE_BYTES: usize = if WIDEST_BUILT_PANEL_LINE_BYTES > WIDEST_TIMERS_BYTES {
    WIDEST_BUILT_PANEL_LINE_BYTES
} else {
    WIDEST_TIMERS_BYTES
};

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

/// A panel the orb can show. On the wire: `"stats"`, `"camera"` or `"built"`.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PanelName {
    Stats,
    /// The camera's live picture. Its frames do not travel in these messages:
    /// the UI fetches them from Jarvis's `/camera` route while this panel shows.
    Camera,
    /// A Panel the model built from the Catalog ([`BuiltPanel`]). It opens with
    /// [`ToOrb::ShowBuiltPanel`], which carries its parts, never by name: no
    /// message or tool argument reads `"built"` as a panel to show.
    #[serde(skip_deserializing)]
    Built,
}

/// A live figure a gauge shows. Jarvis fills in its value from the same
/// [`ToOrb::Stats`] the Dashboard reads; the model only names it. On the wire:
/// `"cpu"`, `"temperature"`, `"ram"`, `"storage"`, `"llama"`, `"bge"`.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GaugeSource {
    Cpu,
    Temperature,
    Ram,
    Storage,
    /// Whether the language server on the 16 GB Pi is up.
    Llama,
    /// Whether the embedding server on the 16 GB Pi is up.
    Bge,
}

/// One part of a built Panel: the Catalog (jarvis-ui 12, G3). Text comes from
/// the model and is shown as text, never as markup. On the wire:
///
/// ```text
/// {"kind":"card","title":"Reminder","text":"Water the plants"}
/// {"kind":"list","title":"Shopping","items":["milk","eggs"]}
/// {"kind":"gauge","source":"cpu"}
/// ```
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PanelPart {
    Card { title: String, text: String },
    List { title: String, items: Vec<String> },
    Gauge { source: GaugeSource },
}

/// A Panel the model built, within every limit by construction. Decoding checks
/// the limits too, so a line over them is invalid.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(try_from = "UncheckedBuiltPanel")]
pub struct BuiltPanel {
    title: String,
    parts: Vec<PanelPart>,
}

/// A built Panel as it arrives, before its limits are checked.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UncheckedBuiltPanel {
    title: String,
    parts: Vec<PanelPart>,
}

/// The first limit a built Panel breaks. Part and item numbers count from 1.
/// The message is what the model is told, so it names the limit.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum BuiltPanelError {
    #[error("a panel title has at most {MAX_PANEL_TITLE_CHARS} characters, not {chars}")]
    TitleTooLong { chars: usize },
    #[error("a panel has at least {MIN_PANEL_PARTS} part")]
    NoParts,
    #[error("a panel has at most {MAX_PANEL_PARTS} parts, not {parts}")]
    TooManyParts { parts: usize },
    #[error("part {part}: a card title has at most {MAX_PART_TEXT_CHARS} characters, not {chars}")]
    CardTitleTooLong { part: usize, chars: usize },
    #[error("part {part}: a card text has at most {MAX_PART_TEXT_CHARS} characters, not {chars}")]
    CardTextTooLong { part: usize, chars: usize },
    #[error("part {part}: a list title has at most {MAX_PART_TEXT_CHARS} characters, not {chars}")]
    ListTitleTooLong { part: usize, chars: usize },
    #[error("part {part}: a list has at least {MIN_LIST_ITEMS} item")]
    EmptyList { part: usize },
    #[error("part {part}: a list has at most {MAX_LIST_ITEMS} items, not {items}")]
    TooManyItems { part: usize, items: usize },
    #[error(
        "part {part}, item {item}: a list item has at most {MAX_PART_TEXT_CHARS} characters, not {chars}"
    )]
    ListItemTooLong {
        part: usize,
        item: usize,
        chars: usize,
    },
}

/// `Some(chars)` when `text` has more than `limit` characters (Unicode scalar
/// values, not bytes).
fn over_limit(text: &str, limit: usize) -> Option<usize> {
    let chars = text.chars().count();
    (chars > limit).then_some(chars)
}

/// Checks one part, numbered `part` from 1.
fn check_part(part: usize, content: &PanelPart) -> Result<(), BuiltPanelError> {
    match content {
        PanelPart::Card { title, text } => {
            if let Some(chars) = over_limit(title, MAX_PART_TEXT_CHARS) {
                return Err(BuiltPanelError::CardTitleTooLong { part, chars });
            }
            if let Some(chars) = over_limit(text, MAX_PART_TEXT_CHARS) {
                return Err(BuiltPanelError::CardTextTooLong { part, chars });
            }
        }
        PanelPart::List { title, items } => {
            if let Some(chars) = over_limit(title, MAX_PART_TEXT_CHARS) {
                return Err(BuiltPanelError::ListTitleTooLong { part, chars });
            }
            if items.len() < MIN_LIST_ITEMS {
                return Err(BuiltPanelError::EmptyList { part });
            }
            if items.len() > MAX_LIST_ITEMS {
                return Err(BuiltPanelError::TooManyItems {
                    part,
                    items: items.len(),
                });
            }
            for (index, item) in items.iter().enumerate() {
                if let Some(chars) = over_limit(item, MAX_PART_TEXT_CHARS) {
                    return Err(BuiltPanelError::ListItemTooLong {
                        part,
                        item: index + 1,
                        chars,
                    });
                }
            }
        }
        PanelPart::Gauge { .. } => {}
    }
    Ok(())
}

impl BuiltPanel {
    /// Checks every limit (G4), title first and then each part in order, and
    /// returns the first one broken.
    pub fn new(title: String, parts: Vec<PanelPart>) -> Result<Self, BuiltPanelError> {
        if let Some(chars) = over_limit(&title, MAX_PANEL_TITLE_CHARS) {
            return Err(BuiltPanelError::TitleTooLong { chars });
        }
        if parts.len() < MIN_PANEL_PARTS {
            return Err(BuiltPanelError::NoParts);
        }
        if parts.len() > MAX_PANEL_PARTS {
            return Err(BuiltPanelError::TooManyParts { parts: parts.len() });
        }
        for (index, part) in parts.iter().enumerate() {
            check_part(index + 1, part)?;
        }
        Ok(Self { title, parts })
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn parts(&self) -> &[PanelPart] {
        &self.parts
    }
}

impl TryFrom<UncheckedBuiltPanel> for BuiltPanel {
    type Error = BuiltPanelError;

    fn try_from(unchecked: UncheckedBuiltPanel) -> Result<Self, Self::Error> {
        Self::new(unchecked.title, unchecked.parts)
    }
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

/// Where a timer is. On the wire: `"pending"` (heard but waiting for a yes),
/// `"running"` or `"ended"`. A cancelled timer is not on the wire: it is simply
/// absent from the list.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TimerState {
    Pending,
    Running,
    Ended,
}

/// One timer on the screen, with a label within [`MAX_TIMER_LABEL_CHARS`] by
/// construction. Decoding checks the limit too. On the wire, with `label`
/// omitted when there is none:
///
/// ```text
/// {"id":3,"label":"Time to sleep","ends_at_ms":1763830800000,"state":"running"}
/// ```
///
/// The id is a `u64`, so a negative id never decodes. It is Jarvis's stored id,
/// which the UI sends back unchanged.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(try_from = "UncheckedTimer")]
pub struct Timer {
    id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    /// Unix milliseconds.
    ends_at_ms: u64,
    state: TimerState,
}

/// A timer as it arrives, before its label is checked.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UncheckedTimer {
    id: u64,
    #[serde(default, deserialize_with = "present_label")]
    label: Option<String>,
    ends_at_ms: u64,
    state: TimerState,
}

/// Reads a `label` that is present: a JSON `null` is invalid. No label is the
/// key being absent.
fn present_label<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    String::deserialize(deserializer).map(Some)
}

/// A timer label over [`MAX_TIMER_LABEL_CHARS`].
#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
#[error("a timer label has at most {MAX_TIMER_LABEL_CHARS} characters, not {chars}")]
pub struct TimerLabelTooLong {
    pub chars: usize,
}

impl Timer {
    pub fn new(
        id: u64,
        label: Option<String>,
        ends_at_ms: u64,
        state: TimerState,
    ) -> Result<Self, TimerLabelTooLong> {
        if let Some(chars) = label
            .as_deref()
            .and_then(|text| over_limit(text, MAX_TIMER_LABEL_CHARS))
        {
            return Err(TimerLabelTooLong { chars });
        }
        Ok(Self {
            id,
            label,
            ends_at_ms,
            state,
        })
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn label(&self) -> Option<&str> {
        self.label.as_deref()
    }

    /// When the timer ends, in unix milliseconds.
    pub fn ends_at_ms(&self) -> u64 {
        self.ends_at_ms
    }

    pub fn state(&self) -> TimerState {
        self.state
    }
}

impl TryFrom<UncheckedTimer> for Timer {
    type Error = TimerLabelTooLong;

    fn try_from(unchecked: UncheckedTimer) -> Result<Self, Self::Error> {
        Self::new(
            unchecked.id,
            unchecked.label,
            unchecked.ends_at_ms,
            unchecked.state,
        )
    }
}

/// Jarvis to the orb.
///
/// ```text
/// {"type":"state","state":"thinking"}
/// {"type":"level","level":0.25,"source":"mic"}
/// {"type":"show_panel","panel":"stats"}
/// {"type":"show_panel","panel":"camera"}
/// {"type":"show_built_panel","title":"Shopping","parts":[{"kind":"list","title":"To buy","items":["milk","eggs"]},{"kind":"gauge","source":"cpu"}]}
/// {"type":"hide_panel"}
/// {"type":"timers","timers":[{"id":3,"label":"Time to sleep","ends_at_ms":1763830800000,"state":"running"}]}
/// {"type":"stats","cpu_percent":12.5,"temperature_c":55.0,"ram_used_bytes":2,"ram_total_bytes":8,"storage":{"available_bytes":207517036544,"total_bytes":251416899584},"llama":"up","bge":"down"}
/// ```
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ToOrb {
    /// Sent on every state change.
    State { state: OrbState },
    /// Sent about 30 times a second while there is something to hear.
    Level { level: Level, source: LevelSource },
    /// Opens a panel over the orb. Never `PanelName::Built`: that one opens
    /// with [`ToOrb::ShowBuiltPanel`].
    ShowPanel { panel: PanelName },
    /// Opens a built Panel over the orb, replacing any panel: its title and
    /// parts and the "show" in one message. `HidePanel` closes it.
    ShowBuiltPanel(BuiltPanel),
    /// Closes whatever panel is open.
    HidePanel {},
    /// Every timer to show, sent on every change and when the UI connects. An
    /// empty list clears the strip.
    Timers { timers: Vec<Timer> },
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
/// {"type":"cancel_timer","id":3}
/// ```
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum FromOrb {
    /// The screen was tapped. It carries nothing: Jarvis decides what a tap
    /// means from its own state.
    Tap {},
    /// Darren tapped outside the open panel, and the orb closed it.
    PanelClosed {},
    /// Darren tapped a timer's cancel (or an ended row). Jarvis checks the id
    /// against its own storage; the UI decides nothing.
    CancelTimer { id: u64 },
    /// Darren answered yes to a pending timer.
    ConfirmTimer { id: u64 },
    /// Darren answered no to a pending timer.
    RejectTimer { id: u64 },
}

#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("line is {len} bytes, over the limit of {limit}")]
    LineTooLong { len: usize, limit: usize },
    #[error("line is not a valid message")]
    Invalid(#[from] serde_json::Error),
}

impl ToOrb {
    /// The message as one line, ending in `\n`, at most [`MAX_TO_ORB_LINE_BYTES`].
    pub fn encode(&self) -> Result<String, ProtocolError> {
        encode_line(self, MAX_TO_ORB_LINE_BYTES)
    }

    /// Reads one line, with or without its `\n`.
    pub fn decode(line: &str) -> Result<Self, ProtocolError> {
        decode_line(line, MAX_TO_ORB_LINE_BYTES)
    }
}

impl FromOrb {
    /// The message as one line, ending in `\n`, at most [`MAX_LINE_BYTES`].
    pub fn encode(&self) -> Result<String, ProtocolError> {
        encode_line(self, MAX_LINE_BYTES)
    }

    /// Reads one line, with or without its `\n`.
    pub fn decode(line: &str) -> Result<Self, ProtocolError> {
        decode_line(line, MAX_LINE_BYTES)
    }
}

fn encode_line<T: Serialize>(message: &T, limit: usize) -> Result<String, ProtocolError> {
    let mut line = serde_json::to_string(message)?;
    if line.len() > limit {
        return Err(ProtocolError::LineTooLong {
            len: line.len(),
            limit,
        });
    }
    line.push('\n');
    Ok(line)
}

fn decode_line<T: DeserializeOwned>(line: &str, limit: usize) -> Result<T, ProtocolError> {
    let line = line.strip_suffix('\n').unwrap_or(line);
    // Checked before parsing, so an oversized line costs no parse.
    if line.len() > limit {
        return Err(ProtocolError::LineTooLong {
            len: line.len(),
            limit,
        });
    }
    Ok(serde_json::from_str(line)?)
}
