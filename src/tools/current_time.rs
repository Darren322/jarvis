use std::convert::Infallible;

use jiff::Zoned;
use rig_agent::tool::{Tool, ToolContext};
use rig_core::serde_json::json;
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
pub struct CurrentTimeArgs {}

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

/// Tells the model the real time, so it never guesses it.
pub(crate) struct CurrentTimeTool;

impl Tool for CurrentTimeTool {
    const NAME: &'static str = "current_time";

    type Args = CurrentTimeArgs;
    type Output = CurrentTime;
    type Error = Infallible;

    fn description(&self) -> String {
        "Returns the current local time, the weekday, the date, the time zone and its UTC \
         offset, read from this Pi's clock. Use it for any question about the current time, \
         date or day of the week, and never guess them. Answer only what was asked: for the \
         time, say just the time as given (\"It's 7:31 PM.\"), and add the weekday, date or \
         time zone only when the question asks for them. If the time zone is \"Etc/Unknown\", \
         say the time zone is unknown and do not guess one."
            .to_string()
    }
    fn parameters(&self) -> rig_core::serde_json::Value {
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    }
    async fn call(
        &self,
        _context: &mut ToolContext,
        _args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        Ok(format_current_time(&Zoned::now()))
    }
}

#[cfg(test)]
#[path = "../../tests/unit/tools/current_time_tests.rs"]
mod tests;
