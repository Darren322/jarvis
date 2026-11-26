use jiff::Zoned;
use rig_agent::tool::{Tool, ToolContext, ToolExecutionError};
use rig_core::serde_json::{Value, json};
use serde::Serialize;

use super::TimerToolError;
use super::resolve::spoken_at;
use crate::features::timers::TimersHandle;
use crate::storage::{Timer, TimerState};

const MS_PER_MINUTE: i64 = 60_000;
const MINUTES_PER_HOUR: i64 = 60;

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListTimersArgs {}

/// One timer as the model is told it, in words it can say.
#[derive(Debug, Eq, PartialEq, Serialize)]
pub(crate) struct TimerView {
    pub(crate) label: Option<String>,
    /// `waiting_for_yes`, `running` or `ended`.
    pub(crate) state: &'static str,
    /// "11:00 pm tomorrow".
    pub(crate) ends_at: String,
    /// "25 minutes"; only for a running timer.
    pub(crate) time_left: Option<String>,
}

/// The timers as the model is told them at `now`.
pub(crate) fn views(now: &Zoned, timers: &[Timer]) -> Result<Vec<TimerView>, TimerToolError> {
    let now_ms = now.timestamp().as_millisecond();
    timers
        .iter()
        .map(|timer| {
            let (state, time_left) = match timer.state {
                TimerState::Pending => ("waiting_for_yes", None),
                TimerState::Running => (
                    "running",
                    Some(time_left(timer.ends_at_unix_ms.saturating_sub(now_ms))),
                ),
                // A cancelled timer is never shown; "ended" is the safe reading.
                TimerState::Ended | TimerState::Cancelled => ("ended", None),
            };
            Ok(TimerView {
                label: timer.label.clone(),
                state,
                ends_at: spoken_at(now, timer.ends_at_unix_ms)?,
                time_left,
            })
        })
        .collect()
}

/// "25 minutes", "1 hour 5 minutes", "2 hours"; whole minutes, rounded down.
fn time_left(left_ms: i64) -> String {
    let minutes = left_ms.max(0) / MS_PER_MINUTE;
    let (hours, minutes) = (minutes / MINUTES_PER_HOUR, minutes % MINUTES_PER_HOUR);
    let unit = |count: i64, name: &str| {
        let plural = if count == 1 { "" } else { "s" };
        format!("{count} {name}{plural}")
    };
    match (hours, minutes) {
        (0, 0) => "less than a minute".to_owned(),
        (0, minutes) => unit(minutes, "minute"),
        (hours, 0) => unit(hours, "hour"),
        (hours, minutes) => format!("{} {}", unit(hours, "hour"), unit(minutes, "minute")),
    }
}

/// Tells the model which timers there are and how long each has left.
pub(crate) struct ListTimersTool {
    timers: TimersHandle,
}

impl ListTimersTool {
    pub(crate) fn new(timers: TimersHandle) -> Self {
        Self { timers }
    }

    fn list(&self, now: &Zoned) -> Result<Value, TimerToolError> {
        let timers = views(now, &self.timers.shown())?;
        if timers.is_empty() {
            return Ok(json!({ "timers": [], "note": "There are no timers." }));
        }
        Ok(json!({
            "timers": timers,
            "note": "Say how long is left on each running timer, using `time_left`. A timer \
                     that is `waiting_for_yes` is not set yet.",
        }))
    }
}

impl Tool for ListTimersTool {
    const NAME: &'static str = "list_timers";

    type Args = ListTimersArgs;
    type Output = Value;
    type Error = TimerToolError;

    fn description(&self) -> String {
        "Lists the timers and how long each has left. Use it when the user asks how long is \
         left or what timers are set, for example \"how long is left\" or \"what timers do I \
         have\". Say only what it returns."
            .to_owned()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    }

    fn map_error(&self, error: Self::Error) -> ToolExecutionError {
        error.for_model()
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        _args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        self.list(&Zoned::now())
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/tools/timers/list_timers_tests.rs"]
mod tests;
