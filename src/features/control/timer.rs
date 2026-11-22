//! Answering a proposed timer ("yes" or "no", timers D4) and every fixed line
//! Jarvis says about timers, in one place.
//!
//! The model never confirms a timer: `set_timer` only proposes one, and only
//! Darren's "yes" here (or a tap on Set, timers 07) makes it run.

use std::time::Duration;

use jiff::{Timestamp, Zoned};

use super::Notice;
use crate::features::timers::{TimerEvent, TimerServiceError, TimersHandle};
use crate::storage::{Timer, TimerError, TimerId, TimerState};
use crate::tools::timers::resolve::{SPOKEN_TIME_FORMAT, spoken_at};

/// Darren's answer to the timer Jarvis just proposed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TimerAnswer {
    Yes,
    No,
}

/// What an answer did.
#[derive(Debug, Eq, PartialEq)]
pub(crate) enum AnswerOutcome {
    /// It answered the pending timer; this is the line to show and say.
    Answered(Notice),
    /// No timer was waiting (or its window had closed): the words go to the model
    /// as an ordinary prompt.
    NotForATimer,
}

/// Answers the one pending timer (D23 keeps it to one) with `answer`.
pub(crate) async fn answer_timer(timers: &TimersHandle, answer: TimerAnswer) -> AnswerOutcome {
    let Some(pending) = timers
        .shown()
        .into_iter()
        .find(|timer| timer.state == TimerState::Pending)
    else {
        return AnswerOutcome::NotForATimer;
    };
    let result = match answer {
        TimerAnswer::Yes => timers.confirm(pending.id).await,
        TimerAnswer::No => timers.reject(pending.id).await,
    };
    match (answer, result) {
        (TimerAnswer::Yes, Ok(timer)) => {
            AnswerOutcome::Answered(Notice::Info(set_line(&timer, &Zoned::now())))
        }
        (TimerAnswer::No, Ok(_)) => AnswerOutcome::Answered(Notice::Info(REJECTED_LINE.to_owned())),
        // Gone between the look and the answer (its window closed, or it was
        // replaced): the words were not for a timer after all.
        (
            _,
            Err(
                TimerServiceError::WindowOver { .. }
                | TimerServiceError::NotPending { .. }
                | TimerServiceError::Timer(_),
            ),
        ) => AnswerOutcome::NotForATimer,
        (_, Err(error @ (TimerServiceError::Unavailable | TimerServiceError::Storage(_)))) => {
            AnswerOutcome::Answered(Notice::Warning(format!(
                "could not answer the timer: {error}"
            )))
        }
    }
}

/// A tap on the screen about one timer. The id is whatever the screen sent: it is
/// checked against the timers, never trusted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TimerTap {
    /// Set, on a proposed timer.
    Confirm(TimerId),
    /// Don't, on a proposed timer.
    Reject(TimerId),
    /// Cancel, on a running timer (after the screen asked "Cancel it?"), or a tap
    /// on an ended row.
    Cancel(TimerId),
}

/// Does what a tap on the screen asks and returns the line to show and say, if
/// there is one (D11). No model run: the taps are fixed answers.
///
/// A tap that changes nothing (an unknown id, a timer in the wrong state, a Set
/// after its 30 seconds) gives no line: the service refuses it and the list the
/// screen is sent stays the truth. Cancelling an ended timer removes its row and
/// says nothing (D17). Only a failure of the service or of storage is a warning.
pub(crate) async fn answer_tap(timers: &TimersHandle, tap: TimerTap) -> Option<Notice> {
    let result = match tap {
        TimerTap::Confirm(id) => timers
            .confirm(id)
            .await
            .map(|timer| Some(set_line(&timer, &Zoned::now()))),
        TimerTap::Reject(id) => timers
            .reject(id)
            .await
            .map(|_| Some(REJECTED_LINE.to_owned())),
        TimerTap::Cancel(id) => cancel_tapped(timers, id).await,
    };
    match result {
        Ok(line) => line.map(Notice::Info),
        Err(error) => refusal_notice(&error),
    }
}

/// Cancels the shown timer `id`. The line is for a timer that was counting or
/// waiting; an ended one is only removed. An id that is not shown is nobody's
/// timer: nothing is cancelled.
async fn cancel_tapped(
    timers: &TimersHandle,
    id: TimerId,
) -> Result<Option<String>, TimerServiceError> {
    let Some(shown) = timers.shown().into_iter().find(|timer| timer.id == id) else {
        return Ok(None);
    };
    let cancelled = timers.cancel(id).await?;
    Ok(match shown.state {
        // `shown` never lists a cancelled timer, and `cancel` refuses one.
        TimerState::Ended | TimerState::Cancelled => None,
        TimerState::Pending | TimerState::Running => Some(cancelled_line(&cancelled)),
    })
}

/// A refused tap is silent; a broken service or storage is worth a warning.
fn refusal_notice(error: &TimerServiceError) -> Option<Notice> {
    match error {
        TimerServiceError::WindowOver { .. }
        | TimerServiceError::NotPending { .. }
        | TimerServiceError::Timer(
            TimerError::NotInExpectedState { .. } | TimerError::LimitReached { .. },
        ) => None,
        TimerServiceError::Unavailable
        | TimerServiceError::Timer(TimerError::Storage(_))
        | TimerServiceError::Storage(_) => Some(Notice::Warning(format!(
            "could not change the timer: {error}"
        ))),
    }
}

/// Said after "no" (or Don't).
pub(crate) const REJECTED_LINE: &str = "Okay, no timer.";

/// Said in place of the model's reply after a run that proposed `timer` (D4, D16).
pub(crate) fn proposal_line(timer: &Timer, now: &Zoned) -> String {
    match when(timer, now) {
        Some(when) => format!("Set {} for {when}? Say yes or tap Set.", name(timer)),
        None => format!("Set {}? Say yes or tap Set.", name(timer)),
    }
}

/// Said when a timer is confirmed.
pub(crate) fn set_line(timer: &Timer, now: &Zoned) -> String {
    match when(timer, now) {
        Some(when) => format!("Set. {} ends at {when}.", capitalised(&name(timer))),
        None => "Set.".to_owned(),
    }
}

/// Said when Darren cancels a timer on the screen.
fn cancelled_line(timer: &Timer) -> String {
    format!("Cancelled {}.", name(timer))
}

/// Said when a proposed timer was never confirmed in its window.
pub(crate) fn expired_line(timer: &Timer) -> String {
    format!("No yes came, so I dropped {}.", name(timer))
}

/// What Jarvis does about a timer event, while idle.
#[derive(Debug, Eq, PartialEq)]
pub(crate) enum TimerReaction {
    /// Ask the model, with no tools, to remind Darren (D6): this is the prompt.
    Remind(String),
    /// Show and say this line.
    Say(Notice),
    /// Only show this line: a timer missed by an hour or more is dropped with a
    /// printed notice (D9), and a storage failure is for the log.
    Show(Notice),
}

/// How Jarvis reacts to `event`, as seen from `now`.
pub(crate) fn react_to(event: &TimerEvent, now: &Zoned) -> TimerReaction {
    match event {
        TimerEvent::Ended { timer, late_by } => {
            TimerReaction::Remind(reminder_prompt(timer, *late_by, now))
        }
        TimerEvent::Missed { timer } => TimerReaction::Show(Notice::Info(format!(
            "{} ended over an hour ago, while I was off, so I let it go.",
            capitalised(&name(timer))
        ))),
        TimerEvent::Expired { timer } => TimerReaction::Say(Notice::Info(expired_line(timer))),
        TimerEvent::Failed(error) => TimerReaction::Show(Notice::Warning(format!(
            "the timers could not be read: {error}"
        ))),
    }
}

/// The prompt of a timer's reminder run (D6): Jarvis's own line, sent in the
/// user's turn, with the label and when the timer was set; a late one says how
/// late (D9). It is archived and joins the history like any prompt.
pub(crate) fn reminder_prompt(timer: &Timer, late_by: Option<Duration>, now: &Zoned) -> String {
    let what = match timer.label.as_deref().map(str::trim) {
        Some(label) if !label.is_empty() => format!("My timer '{label}'"),
        _ => "My timer".to_owned(),
    };
    let set = Timestamp::from_millisecond(timer.created_at_unix_ms)
        .map(|set| {
            let set = set.to_zoned(now.time_zone().clone());
            format!(", set at {},", set.strftime(SPOKEN_TIME_FORMAT))
        })
        .unwrap_or_default();
    let ended = match late_by {
        None => "has just ended".to_owned(),
        Some(late_by) => format!("ended {} ago, while you were off", minutes_ago(late_by)),
    };
    format!("{what}{set} {ended}. Remind me in a sentence or two.")
}

/// "40 minutes", "1 minute", or "less than a minute".
fn minutes_ago(late_by: Duration) -> String {
    match late_by.as_secs() / 60 {
        0 => "less than a minute".to_owned(),
        1 => "1 minute".to_owned(),
        minutes => format!("{minutes} minutes"),
    }
}

/// "'Time to sleep'" for a labelled timer, "the timer" otherwise.
fn name(timer: &Timer) -> String {
    match timer.label.as_deref().map(str::trim) {
        Some(label) if !label.is_empty() => format!("'{label}'"),
        _ => "the timer".to_owned(),
    }
}

fn capitalised(text: &str) -> String {
    let mut characters = text.chars();
    match characters.next() {
        Some(first) => first.to_uppercase().chain(characters).collect(),
        None => String::new(),
    }
}

/// "11:00 pm tomorrow" as seen from `now`, or `None` if the end cannot be read
/// as a time.
fn when(timer: &Timer, now: &Zoned) -> Option<String> {
    spoken_at(now, timer.ends_at_unix_ms).ok()
}

#[cfg(test)]
#[path = "../../../tests/unit/features/control/timer_tests.rs"]
mod tests;
