use std::convert::Infallible;

use jiff::Zoned;
use rig_agent::tool::{Tool, ToolContext};
use rig_core::serde_json::{Value, json};
use serde::Serialize;

/// 12-hour, unpadded hour, as it is said: `7:31 PM`, `12:00 AM`. Darren heard
/// "19:31" read out on 2026-11-21 and wants the spoken 12-hour form.
const TIME_FORMAT: &str = "%-I:%M %p";
const WEEKDAY_FORMAT: &str = "%A";
/// As it is said: `21 November 2026`, not an ISO date read digit by digit.
const DATE_FORMAT: &str = "%-d %B %Y";
/// `%:z` prints the offset with a colon and minutes, e.g. `+08:00` or `-03:30`.
const UTC_OFFSET_FORMAT: &str = "%:z";
/// What jiff itself calls the zone it falls back to when the system zone cannot
/// be read. `TimeZone::iana_name` returns `None` for it, so the name is spelled here.
const UNKNOWN_TIME_ZONE: &str = "Etc/Unknown";

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CurrentTimeArgs {
    /// Which part of the reading the question asks for. The schema requires it;
    /// a call without it still gets the whole reading rather than failing.
    #[serde(default)]
    pub part: TimePart,
}

/// What the model may ask the clock for. The tool returns only that part,
/// because a small model says everything it is given: told the date for "what
/// time is it", it read the date out too (Darren, 2026-11-21).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimePart {
    /// The time of day only.
    Time,
    /// The weekday and the date only.
    Date,
    /// Everything, with the time zone.
    #[default]
    All,
}

/// The clock reading the model is told, in the Pi's own time zone.
#[derive(Debug, PartialEq, Eq, Serialize)]
pub(crate) struct CurrentTime {
    /// 12-hour `7:31 PM`, no seconds.
    pub(crate) time: String,
    /// English weekday name.
    pub(crate) weekday: String,
    /// `21 November 2026`.
    pub(crate) date: String,
    /// IANA name, or `Etc/Unknown` when the system zone could not be read.
    pub(crate) time_zone: String,
    /// `+08:00` style.
    pub(crate) utc_offset: String,
}

/// Formats one instant; pure so tests pass fixed values instead of a clock.
pub(crate) fn format_current_time(now: &Zoned) -> CurrentTime {
    CurrentTime {
        time: now.strftime(TIME_FORMAT).to_string(),
        weekday: now.strftime(WEEKDAY_FORMAT).to_string(),
        date: now.strftime(DATE_FORMAT).to_string(),
        time_zone: now
            .time_zone()
            .iana_name()
            .unwrap_or(UNKNOWN_TIME_ZONE)
            .to_string(),
        utc_offset: now.strftime(UTC_OFFSET_FORMAT).to_string(),
    }
}

/// The part of `reading` that `part` asks for, as the tool's JSON output.
pub(crate) fn answer(reading: CurrentTime, part: TimePart) -> Value {
    match part {
        TimePart::Time => json!({ "time": reading.time }),
        TimePart::Date => json!({ "weekday": reading.weekday, "date": reading.date }),
        TimePart::All => json!(reading),
    }
}

/// Tells the model the real time, so it never guesses it.
pub(crate) struct CurrentTimeTool;

impl Tool for CurrentTimeTool {
    const NAME: &'static str = "current_time";

    type Args = CurrentTimeArgs;
    type Output = Value;
    type Error = Infallible;

    fn description(&self) -> String {
        "Reads this Pi's clock. Use it for any question about the current time, date or day \
         of the week, and never guess them. Set `part` to what the question asks for: \
         \"time\" for the time of day (\"what time is it\"), \"date\" for the day or date \
         (\"what day is it\", \"what's the date\"), \"all\" only when both are asked or the \
         time zone is. Say only what it returns, e.g. \"It's 7:31 PM.\" If the time zone is \
         \"Etc/Unknown\", say the time zone is unknown and do not guess one."
            .to_string()
    }
    fn parameters(&self) -> rig_core::serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "part": {
                    "type": "string",
                    "enum": ["time", "date", "all"],
                    "description": "What the question asks for."
                }
            },
            "required": ["part"],
            "additionalProperties": false
        })
    }
    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        Ok(answer(format_current_time(&Zoned::now()), args.part))
    }
}

#[cfg(test)]
#[path = "../../tests/unit/tools/current_time_tests.rs"]
mod tests;
