use std::convert::Infallible;

use jiff::Zoned;
use rig_agent::tool::{Tool, ToolContext};
use rig_core::serde_json::json;
use serde::Serialize;

const TIME_FORMAT: &str = "%H:%M";
const WEEKDAY_FORMAT: &str = "%A";
const DATE_FORMAT: &str = "%Y-%m-%d";
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
    /// 24-hour `HH:MM`, no seconds.
    pub(crate) time: String,
    /// English weekday name.
    pub(crate) weekday: String,
    /// ISO `YYYY-MM-DD`.
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
        "Returns the current local time (24-hour), the weekday, the date, the time zone and its \
         UTC offset, read from this Pi's clock. Use it for any question about the current time, \
         date or day of the week, and never guess them. If the time zone is \"Etc/Unknown\", \
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
