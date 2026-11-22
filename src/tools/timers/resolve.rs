//! When a timer ends (timers D2, D18), as pure functions of `jiff` values so tests
//! pass fixed instants and zones instead of a clock.

use jiff::{Span, Timestamp, Zoned};
use thiserror::Error;

/// The fewest minutes a timer may last. A zero-minute timer would end before
/// Darren could confirm it.
pub(crate) const MIN_TIMER_MINUTES: i64 = 1;

/// The most minutes a timer may last: a day. A longer wait is a calendar's job.
pub(crate) const MAX_TIMER_MINUTES: i64 = 24 * 60;

/// 12-hour, unpadded hour, lowercase `pm`, as Jarvis says it ("11:00 pm").
pub(crate) const SPOKEN_TIME_FORMAT: &str = "%-I:%M %P";
/// For a day that is neither today nor tomorrow: "Monday 23 November".
const SPOKEN_DAY_FORMAT: &str = "%A %-d %B";

/// What the model asked for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TimeRequest<'a> {
    /// That many minutes from now, [`MIN_TIMER_MINUTES`] to [`MAX_TIMER_MINUTES`].
    InMinutes(i64),
    /// A 24-hour `"HH:MM"`: today if it is still ahead, tomorrow otherwise.
    AtClock(&'a str),
}

/// When the timer ends.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct ResolvedTime {
    pub(crate) ends_at_unix_ms: i64,
    /// As said aloud: "11:00 pm tomorrow".
    pub(crate) spoken: String,
}

/// Why a request could not be turned into an end time. Each message tells the
/// model what to fix.
#[derive(Debug, Error)]
pub(crate) enum ResolveError {
    #[error(
        "{minutes} minutes is not allowed: `minutes` is a whole number from {MIN_TIMER_MINUTES} \
         to {MAX_TIMER_MINUTES} (24 hours); for a time of day use `at` instead"
    )]
    MinutesOutOfRange { minutes: i64 },
    #[error(
        "`at` must be a 24-hour time written HH:MM, such as \"23:00\" for 11 pm or \"07:30\" for \
         7:30 am, not {given:?}"
    )]
    NotAClockTime { given: String },
    #[error("the end time could not be worked out from the clock")]
    Clock(#[from] jiff::Error),
}

/// The end instant for `request`, asked at `now`. A clock time equal to or
/// before `now` (to the minute) means tomorrow.
///
/// A clock time is built as a civil time in `now`'s zone, never as 24 hours
/// added, so a day with a daylight-saving change still ends at the time asked
/// for. A time the clocks skip is moved forward by jiff's compatible rule.
pub(crate) fn resolve(now: &Zoned, request: TimeRequest<'_>) -> Result<ResolvedTime, ResolveError> {
    let end = match request {
        TimeRequest::InMinutes(minutes) => {
            if !(MIN_TIMER_MINUTES..=MAX_TIMER_MINUTES).contains(&minutes) {
                return Err(ResolveError::MinutesOutOfRange { minutes });
            }
            now.checked_add(Span::new().minutes(minutes))?
        }
        TimeRequest::AtClock(at) => {
            let (hour, minute) = parse_clock_time(at)?;
            let today = now
                .date()
                .at(hour, minute, 0, 0)
                .to_zoned(now.time_zone().clone())?;
            if today.timestamp() > now.timestamp() {
                today
            } else {
                now.date()
                    .tomorrow()?
                    .at(hour, minute, 0, 0)
                    .to_zoned(now.time_zone().clone())?
            }
        }
    };
    Ok(ResolvedTime {
        ends_at_unix_ms: end.timestamp().as_millisecond(),
        spoken: speak(now, &end),
    })
}

/// The spoken form of `ends_at_unix_ms`, in `now`'s zone, relative to `now`'s day.
pub(crate) fn spoken_at(now: &Zoned, ends_at_unix_ms: i64) -> Result<String, ResolveError> {
    let end = Timestamp::from_millisecond(ends_at_unix_ms)?.to_zoned(now.time_zone().clone());
    Ok(speak(now, &end))
}

fn speak(now: &Zoned, end: &Zoned) -> String {
    let time = end.strftime(SPOKEN_TIME_FORMAT);
    if end.date() == now.date() {
        return format!("{time} today");
    }
    if now
        .date()
        .tomorrow()
        .is_ok_and(|tomorrow| end.date() == tomorrow)
    {
        return format!("{time} tomorrow");
    }
    format!("{time} on {}", end.strftime(SPOKEN_DAY_FORMAT))
}

/// Strictly two digits, a colon, two digits: hour 00 to 23, minute 00 to 59.
fn parse_clock_time(at: &str) -> Result<(i8, i8), ResolveError> {
    let not_a_clock_time = || ResolveError::NotAClockTime {
        given: at.to_owned(),
    };
    let (hour, minute) = at.trim().split_once(':').ok_or_else(not_a_clock_time)?;
    let two_digits = |text: &str| text.len() == 2 && text.bytes().all(|byte| byte.is_ascii_digit());
    if !two_digits(hour) || !two_digits(minute) {
        return Err(not_a_clock_time());
    }
    let hour: i8 = hour.parse().map_err(|_| not_a_clock_time())?;
    let minute: i8 = minute.parse().map_err(|_| not_a_clock_time())?;
    if hour > 23 || minute > 59 {
        return Err(not_a_clock_time());
    }
    Ok((hour, minute))
}

#[cfg(test)]
#[path = "../../../tests/unit/tools/timers/resolve_tests.rs"]
mod tests;
