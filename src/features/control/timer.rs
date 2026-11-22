//! Answering a proposed timer ("yes" or "no", timers D4) and every fixed line
//! Jarvis says about timers, in one place.
//!
//! The model never confirms a timer: `set_timer` only proposes one, and only
//! Darren's "yes" here (or a tap on Set, timers 07) makes it run.

use std::time::Duration;

use jiff::{Timestamp, Zoned};

use super::Notice;
use crate::features::timers::{TimerEvent, TimerServiceError, TimersHandle};
use crate::storage::{Timer, TimerState};
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
