use jiff::Zoned;
use orb_protocol::MAX_TIMER_LABEL_CHARS;
use rig_agent::tool::{Tool, ToolContext, ToolExecutionError};
use rig_core::serde_json::{Value, json};
use serde::Deserialize;
use thiserror::Error;

use super::TimerToolError;
use super::resolve::{MAX_TIMER_MINUTES, MIN_TIMER_MINUTES, TimeRequest, resolve};
use crate::features::timers::{TIMER_CONFIRM_WINDOW, TimersHandle};

/// What the model wrote, before it is turned into one end time. Only
/// [`SetTimerArgs`] is ever used.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSetTimerArgs {
    label: Option<String>,
    minutes: Option<i64>,
    seconds: Option<i64>,
    at: Option<String>,
}

/// When the timer should end, as the model said it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TimerWhen {
    InMinutes(i64),
    /// Only so a request in seconds reaches the tool and is answered: a model with
    /// no field for "five seconds" skipped the tool and said the timer was set
    /// (seen on the Pi on 2026-11-26).
    InSeconds(i64),
    /// 24-hour `"HH:MM"`; checked when it is resolved.
    AtClock(String),
}

/// Arguments of `set_timer`. Giving none of `minutes`, `seconds` and `at` fails
/// here, so the policy rejects such a call before it runs.
///
/// More than one is not refused, because a refused call fails the whole run and
/// Gemma sent `{"at": "23:00", "minutes": 120}` for "set a timer for 11 pm" (seen on
/// the Pi on 2026-11-26). `at` wins, since a clock time is what the user said; else
/// `minutes` and `seconds` add up, and the sum must still be whole minutes.
#[derive(Debug, Deserialize)]
#[serde(try_from = "RawSetTimerArgs")]
pub struct SetTimerArgs {
    pub label: Option<String>,
    pub when: TimerWhen,
}

#[derive(Debug, Error)]
pub enum SetTimerArgsError {
    #[error("give one of `minutes`, `seconds` or `at`: none was given")]
    NoTime,
}

impl TryFrom<RawSetTimerArgs> for SetTimerArgs {
    type Error = SetTimerArgsError;

    fn try_from(raw: RawSetTimerArgs) -> Result<Self, Self::Error> {
        let when = match (raw.minutes, raw.seconds, raw.at) {
            (_, _, Some(at)) => TimerWhen::AtClock(at),
            (Some(minutes), None, None) => TimerWhen::InMinutes(minutes),
            (None, Some(seconds), None) => TimerWhen::InSeconds(seconds),
            (Some(minutes), Some(seconds), None) => {
                TimerWhen::InSeconds(minutes.saturating_mul(60).saturating_add(seconds))
            }
            (None, None, None) => return Err(SetTimerArgsError::NoTime),
        };
        Ok(Self {
            label: raw.label,
            when,
        })
    }
}

/// Proposes a timer. It never sets one: Darren's "yes" or a tap on Set does
/// (D4), and the model must not say it is set until then.
pub(crate) struct SetTimerTool {
    timers: TimersHandle,
}

impl SetTimerTool {
    pub(crate) fn new(timers: TimersHandle) -> Self {
        Self { timers }
    }

    /// Checks the arguments, then proposes the timer. `now` is the instant the
    /// request is resolved against.
    async fn propose(&self, now: &Zoned, args: SetTimerArgs) -> Result<Value, TimerToolError> {
        let label = clean_label(args.label)?;
        let request = match &args.when {
            TimerWhen::InMinutes(minutes) => TimeRequest::InMinutes(*minutes),
            TimerWhen::InSeconds(seconds) => TimeRequest::InSeconds(*seconds),
            TimerWhen::AtClock(at) => TimeRequest::AtClock(at),
        };
        let resolved = resolve(now, request)?;
        let timer = self.timers.propose(label, resolved.ends_at_unix_ms).await?;
        Ok(json!({
            "status": "proposed_not_set_yet",
            "label": timer.label,
            "ends_at": resolved.spoken,
            "note": format!(
                "The timer is NOT set yet. It is set only when the user says yes or taps Set, \
                 within {} seconds. Tell the user what you proposed, using the time in \
                 `ends_at`, and that you are waiting for their yes. Never say it is set.",
                TIMER_CONFIRM_WINDOW.as_secs()
            ),
        }))
    }
}

/// The label trimmed, with a capital first letter; none when it is empty or only
/// restates the time ("11 pm", "10 minutes"), which Gemma sends when the user gave
/// no reason (seen on the Pi on 2026-11-26). Refused when it has more than
/// [`MAX_TIMER_LABEL_CHARS`] characters, so one stored label can never make the
/// screen's `timers` message unsendable.
fn clean_label(label: Option<String>) -> Result<Option<String>, TimerToolError> {
    let Some(label) = label else {
        return Ok(None);
    };
    let label = label.trim();
    if label.is_empty() || only_restates_the_time(label) {
        return Ok(None);
    }
    let mut letters = label.chars();
    let label: String = letters
        .next()
        .map(|first| first.to_uppercase().chain(letters).collect())
        .unwrap_or_default();
    // Counted after the capital, which can be longer ("ß" becomes "SS").
    let chars = label.chars().count();
    if chars > MAX_TIMER_LABEL_CHARS {
        return Err(TimerToolError::LabelTooLong { chars });
    }
    Ok(Some(label))
}

/// Words that say how long or when, and nothing about what the timer is for.
const TIME_WORDS: &[&str] = &[
    "a", "an", "at", "for", "in", "timer", "minute", "minutes", "min", "mins", "second", "seconds",
    "sec", "secs", "hour", "hours", "half", "am", "pm", "o", "clock", "oclock", "one", "two",
    "three", "four", "five", "six", "seven", "eight", "nine", "ten", "eleven", "twelve", "fifteen",
    "twenty", "thirty", "forty", "fifty", "sixty",
];

/// Whether every word of `label` is a number or a [`TIME_WORDS`] word.
fn only_restates_the_time(label: &str) -> bool {
    label
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .all(|word| {
            word.bytes().all(|byte| byte.is_ascii_digit())
                || TIME_WORDS
                    .iter()
                    .any(|time_word| word.eq_ignore_ascii_case(time_word))
        })
}

impl Tool for SetTimerTool {
    const NAME: &'static str = "set_timer";

    type Args = SetTimerArgs;
    type Output = Value;
    type Error = TimerToolError;

    /// Says only when to call it and how to fill the arguments. What to say afterwards
    /// is in the result's `note`: with "say ..." lines here, Gemma said them without
    /// calling the tool (seen on the Pi on 2026-11-26). D4 holds either way, because
    /// this tool can only propose.
    fn description(&self) -> String {
        format!(
            "Puts a timer on the screen, where it waits for the user's yes. Use it for every \
             request to set a timer, and never answer one without it: \"set a timer for 10 \
             minutes\", \"put a 10-minute timer on the screen\", \"in 25 minutes, time to do \
             work\", \"set a timer for 30 seconds\", \"set a timer for 11 pm\". Give `label` \
             only when the user says what the timer is for, in their words (for \"in 25 \
             minutes, time to do work\" it is \"Time to do work\"), and leave it out \
             otherwise. Give exactly one of \
             `minutes` (a whole number from {MIN_TIMER_MINUTES} to {MAX_TIMER_MINUTES}), \
             `seconds` (only when the user says seconds) or `at` (a 24-hour HH:MM clock time: \
             turn \"11 pm\" into \"23:00\" and \"7:30 am\" into \"07:30\")."
        )
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "label": {
                    "type": "string",
                    "description": "What the timer is for, in the user's own words. Leave it \
                                    out when the user gave no reason."
                },
                "minutes": {
                    "type": "integer",
                    "description": format!(
                        "A length of time in whole minutes, {MIN_TIMER_MINUTES} to \
                         {MAX_TIMER_MINUTES}. Not together with `seconds` or `at`."
                    )
                },
                "seconds": {
                    "type": "integer",
                    "description": "A length of time in seconds, only when the user says \
                                    seconds. Not together with `minutes` or `at`."
                },
                "at": {
                    "type": "string",
                    "description": "A clock time as 24-hour HH:MM, such as \"23:00\" for 11 pm. \
                                    Not together with `minutes`."
                }
            },
            "additionalProperties": false
        })
    }

    fn map_error(&self, error: Self::Error) -> ToolExecutionError {
        error.for_model()
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        self.propose(&Zoned::now(), args).await
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/tools/timers/set_timer_tests.rs"]
mod tests;
