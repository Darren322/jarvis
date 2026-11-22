use crate::storage::{Timer, TimerId, TimerState};

use super::{Matches, matching};

fn timer(id: i64, label: Option<&str>, state: TimerState) -> Timer {
    Timer {
        id: TimerId(id),
        label: label.map(str::to_owned),
        ends_at_unix_ms: 0,
        created_at_unix_ms: 0,
        state,
    }
}

fn ids(timers: &[Timer]) -> Vec<i64> {
    timers.iter().map(|timer| timer.id.0).collect()
}

/// What a cancel picked, as ids: `One`, `None` or `Several` with their timers.
type Outcome = (&'static str, Vec<i64>);

/// A case: its name, the shown timers, the spoken words, and the outcome.
type Case = (&'static str, Vec<Timer>, Option<&'static str>, Outcome);

fn outcome(matches: Matches) -> Outcome {
    match matches {
        Matches::One(timer) => ("one", vec![timer.id.0]),
        Matches::None(cancellable) => ("none", ids(&cancellable)),
        Matches::Several(matched) => ("several", ids(&matched)),
    }
}

#[test]
fn a_cancel_picks_the_one_timer_it_names_or_lists_the_choices() {
    let sleep = timer(1, Some("Time to sleep"), TimerState::Running);
    let work = timer(2, Some("Time to do work"), TimerState::Running);
    let tea = timer(3, None, TimerState::Pending);
    let ended = timer(4, Some("Sleep check"), TimerState::Ended);
    let cancelled = timer(5, Some("Sleep again"), TimerState::Cancelled);
    let all = [sleep.clone(), work.clone(), tea.clone(), ended, cancelled];

    let cases: [Case; 10] = [
        (
            "T4: no words and one timer",
            vec![sleep.clone()],
            None,
            ("one", vec![1]),
        ),
        (
            "filler words only count as no words",
            vec![sleep.clone()],
            Some("the timer"),
            ("one", vec![1]),
        ),
        (
            "no words and several timers",
            all.to_vec(),
            None,
            ("several", vec![1, 2, 3]),
        ),
        (
            "T4: the sleep one",
            all.to_vec(),
            Some("the sleep one"),
            ("one", vec![1]),
        ),
        (
            "case does not matter",
            all.to_vec(),
            Some("SLEEP"),
            ("one", vec![1]),
        ),
        (
            "every word must appear",
            all.to_vec(),
            Some("do work"),
            ("one", vec![2]),
        ),
        (
            "a word in both labels",
            all.to_vec(),
            Some("time"),
            ("several", vec![1, 2]),
        ),
        (
            "no match lists what could be cancelled",
            all.to_vec(),
            Some("oven"),
            ("none", vec![1, 2, 3]),
        ),
        (
            "an ended or cancelled timer is never cancelled by voice",
            vec![timer(4, Some("Sleep check"), TimerState::Ended)],
            Some("sleep"),
            ("none", vec![]),
        ),
        (
            "an unlabelled timer only matches no words",
            vec![tea],
            Some("tea"),
            ("none", vec![3]),
        ),
    ];
    for (case, shown, words, expected) in cases {
        assert_eq!(outcome(matching(&shown, words)), expected, "{case}");
    }
}
