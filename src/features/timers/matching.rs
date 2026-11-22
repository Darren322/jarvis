//! Which timer a spoken "cancel" means (D10), as a pure function.
//!
//! "cancel the timer" passes no words; "cancel the sleep one" passes "sleep".
//! Only pending and running timers can be cancelled this way: an ended one is
//! already over.

use crate::storage::{Timer, TimerState};

/// Words that say "a timer" rather than which one; they never narrow a match.
const FILLER_WORDS: &[&str] = &["a", "an", "my", "one", "the", "timer", "timers"];

/// The timers a cancel by `words` picks.
#[derive(Debug, Eq, PartialEq)]
pub(crate) enum Matches {
    /// Exactly one timer: cancel it.
    One(Timer),
    /// No timer matched; these are the ones that could be cancelled.
    None(Vec<Timer>),
    /// More than one matched; these are the ones that matched.
    Several(Vec<Timer>),
}

/// The pending and running timers in `shown` that `words` names. With no words
/// (or only filler words), every such timer matches. Otherwise a timer matches
/// when every remaining word appears in its label, ignoring case.
pub(crate) fn matching(shown: &[Timer], words: Option<&str>) -> Matches {
    let cancellable: Vec<&Timer> = shown
        .iter()
        .filter(|timer| matches!(timer.state, TimerState::Pending | TimerState::Running))
        .collect();
    let wanted = words.map(meaningful_words).unwrap_or_default();
    let matched: Vec<Timer> = cancellable
        .iter()
        .filter(|timer| is_named_by(timer, &wanted))
        .map(|timer| (*timer).clone())
        .collect();
    match matched.len() {
        0 => Matches::None(cancellable.into_iter().cloned().collect()),
        1 => Matches::One(matched.into_iter().next().expect("one match was counted")),
        _ => Matches::Several(matched),
    }
}

/// The lowercased words of `text` that are not filler.
fn meaningful_words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .filter(|word| !FILLER_WORDS.contains(&word.as_str()))
        .collect()
}

fn is_named_by(timer: &Timer, wanted: &[String]) -> bool {
    if wanted.is_empty() {
        return true;
    }
    let Some(label) = timer.label.as_deref() else {
        return false;
    };
    let label = label.to_lowercase();
    wanted.iter().all(|word| label.contains(word.as_str()))
}

#[cfg(test)]
#[path = "../../../tests/unit/features/timers/matching_tests.rs"]
mod tests;
