use jiff::Zoned;
use rig_agent::tool::{Tool, ToolContext, ToolExecutionError};
use rig_core::serde_json::{Value, json};

use super::TimerToolError;
use super::list_timers::views;
use crate::features::timers::{CancelOutcome, TimersHandle};

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CancelTimerArgs {
    /// Words from the label of the timer to cancel.
    #[serde(default)]
    pub label: Option<String>,
}

/// Cancels a timer without asking (timers D10). When no single timer is meant,
/// it cancels nothing and tells the model which timers there are.
pub(crate) struct CancelTimerTool {
    timers: TimersHandle,
}

impl CancelTimerTool {
    pub(crate) fn new(timers: TimersHandle) -> Self {
        Self { timers }
    }

    async fn cancel(&self, now: &Zoned, args: CancelTimerArgs) -> Result<Value, TimerToolError> {
        let words = args.label.filter(|label| !label.trim().is_empty());
        Ok(match self.timers.cancel_matching(words).await? {
            CancelOutcome::Cancelled(timer) => json!({
                "status": "cancelled",
                "label": timer.label,
                "note": "That timer is cancelled. Say so in a few words.",
            }),
            CancelOutcome::NoMatch(timers) => json!({
                "status": "no_timer_matched_nothing_cancelled",
                "timers": views(now, &timers)?,
                "note": "No timer matched, so nothing was cancelled. Tell the user which \
                         timers there are and ask which one to cancel.",
            }),
            CancelOutcome::Several(timers) => json!({
                "status": "several_timers_matched_nothing_cancelled",
                "timers": views(now, &timers)?,
                "note": "Several timers matched, so nothing was cancelled. Tell the user which \
                         ones and ask which one to cancel.",
            }),
        })
    }
}

impl Tool for CancelTimerTool {
    const NAME: &'static str = "cancel_timer";

    type Args = CancelTimerArgs;
    type Output = Value;
    type Error = TimerToolError;

    fn description(&self) -> String {
        "Cancels a timer. Use it when the user asks to cancel, stop or remove a timer, for \
         example \"cancel the timer\" or \"cancel the sleep one\". Give `label` with words \
         from the label of the timer they mean, or leave it out when they did not say which \
         (it then cancels the only timer there is). It does not ask the user first. If it \
         says nothing was cancelled, tell the user which timers there are and ask which one."
            .to_owned()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "label": {
                    "type": "string",
                    "description": "Words from the label of the timer to cancel, such as \
                                    \"sleep\". Leave it out when the user did not say which."
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
        self.cancel(&Zoned::now(), args).await
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/tools/timers/cancel_timer_tests.rs"]
mod tests;
