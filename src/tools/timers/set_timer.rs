use jiff::Zoned;
use orb_protocol::MAX_TIMER_LABEL_CHARS;
use rig_agent::tool::{Tool, ToolContext};
use rig_core::serde_json::{Value, json};
use serde::Deserialize;
use thiserror::Error;

use super::TimerToolError;
use super::resolve::{MAX_TIMER_MINUTES, MIN_TIMER_MINUTES, TimeRequest, resolve};
use crate::features::timers::{TIMER_CONFIRM_WINDOW, TimersHandle};

/// What the model wrote, before the rule "exactly one of `minutes` or `at`" is
/// applied. Only [`SetTimerArgs`] is ever used.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSetTimerArgs {
    label: Option<String>,
    minutes: Option<i64>,
    at: Option<String>,
}

/// When the timer should end, as the model said it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TimerWhen {
    InMinutes(i64),
    /// 24-hour `"HH:MM"`; checked when it is resolved.
    AtClock(String),
}

/// Arguments of `set_timer`. Giving neither or both of `minutes` and `at`
/// fails here, so the policy rejects such a call before it runs.
#[derive(Debug, Deserialize)]
#[serde(try_from = "RawSetTimerArgs")]
pub struct SetTimerArgs {
    pub label: Option<String>,
    pub when: TimerWhen,
}

#[derive(Debug, Error)]
pub enum SetTimerArgsError {
    #[error("give exactly one of `minutes` or `at`: both were given")]
    BothMinutesAndAt,
    #[error("give exactly one of `minutes` or `at`: neither was given")]
    NeitherMinutesNorAt,
}

impl TryFrom<RawSetTimerArgs> for SetTimerArgs {
    type Error = SetTimerArgsError;

    fn try_from(raw: RawSetTimerArgs) -> Result<Self, Self::Error> {
        let when = match (raw.minutes, raw.at) {
            (Some(minutes), None) => TimerWhen::InMinutes(minutes),
            (None, Some(at)) => TimerWhen::AtClock(at),
            (Some(_), Some(_)) => return Err(SetTimerArgsError::BothMinutesAndAt),
            (None, None) => return Err(SetTimerArgsError::NeitherMinutesNorAt),
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

/// The label trimmed; none when it is empty. Refused when it has more than
/// [`MAX_TIMER_LABEL_CHARS`] characters, so one stored label can never make
/// the screen's `timers` message unsendable.
fn clean_label(label: Option<String>) -> Result<Option<String>, TimerToolError> {
    let Some(label) = label else {
        return Ok(None);
    };
    let label = label.trim();
    if label.is_empty() {
        return Ok(None);
    }
    let chars = label.chars().count();
    if chars > MAX_TIMER_LABEL_CHARS {
        return Err(TimerToolError::LabelTooLong { chars });
    }
    Ok(Some(label.to_owned()))
}

impl Tool for SetTimerTool {
    const NAME: &'static str = "set_timer";

    type Args = SetTimerArgs;
    type Output = Value;
    type Error = TimerToolError;

    fn description(&self) -> String {
        format!(
            "Proposes a timer, for a length of time (\"set a timer for 10 minutes\", \"put a \
             10-minute timer on the screen\", \"in 25 minutes, time to do work\") or for a clock \
             time (\"set a timer for 11 pm, time to sleep\"). Give `label` when the user says \
             what the timer is for, in a few words (\"Time to sleep\", \"Time to do work\"), and \
             leave it out otherwise. Give exactly one of `minutes` (a whole number from \
             {MIN_TIMER_MINUTES} to {MAX_TIMER_MINUTES}, for a length of time) or `at` (a \
             24-hour HH:MM clock time: turn \"11 pm\" into \"23:00\" and \"7:30 am\" into \
             \"07:30\"). This only proposes the timer. It is NOT set until the user says yes or \
             taps Set, so never say it is set: say the time this tool returns and that you are \
             waiting for their yes. If the tool says no more timers can be added, say so."
        )
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "label": {
                    "type": "string",
                    "description": "What the timer is for, in a few words, such as \"Time to \
                                    sleep\". Leave it out when the user gave no reason."
                },
                "minutes": {
                    "type": "integer",
                    "description": format!(
                        "A length of time in whole minutes, {MIN_TIMER_MINUTES} to \
                         {MAX_TIMER_MINUTES}. Not together with `at`."
                    )
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
