use std::time::Duration;

use crate::storage::{Timer, TimerId, TimerState};

use super::{Overdue, Pass, expired_created_through, is_window_over, next_wake, overdue, pass};

use TimerState::{Cancelled, Ended, Pending, Running};

fn timer(id: i64, state: TimerState, created_at: i64, ends_at: i64) -> Timer {
    Timer {
        id: TimerId(id),
        label: None,
        ends_at_unix_ms: ends_at,
        created_at_unix_ms: created_at,
        state,
    }
}

#[test]
fn a_pending_timer_can_be_confirmed_at_30_seconds_and_not_one_millisecond_later() {
    let proposed = timer(1, Pending, 1_000, 999_999);
    for (now, is_over) in [
        (1_000, false),
        (31_000, false),
        (31_001, true),
        (90_000, true),
    ] {
        assert_eq!(is_window_over(&proposed, now), is_over, "at {now}");
        let Pass { expired, due } = pass(std::slice::from_ref(&proposed), now);
        assert_eq!(expired.len(), usize::from(is_over), "at {now}");
        assert!(due.is_empty());
    }
}

#[test]
fn the_inclusive_delete_takes_exactly_the_timers_whose_window_is_over() {
    // `delete_pending_created_through` deletes `created_at <= through`.
    for (created_at, now, is_deleted) in [(1_000, 31_000, false), (1_000, 31_001, true)] {
        assert_eq!(
            created_at <= expired_created_through(now),
            is_deleted,
            "created {created_at}, now {now}"
        );
        assert_eq!(
            is_window_over(&timer(1, Pending, created_at, 0), now),
            is_deleted
        );
    }
}

#[test]
fn a_running_timer_is_due_at_its_end_and_not_before() {
    let running = timer(1, Running, 0, 60_000);
    for (now, is_due) in [(59_999, false), (60_000, true), (60_001, true)] {
        let Pass { expired, due } = pass(std::slice::from_ref(&running), now);
        assert_eq!(due.len(), usize::from(is_due), "at {now}");
        assert!(expired.is_empty());
    }
}

#[test]
fn only_pending_timers_expire_and_only_running_ones_fall_due() {
    let shown = [
        timer(1, Ended, 0, 0),
        timer(2, Cancelled, 0, 0),
        timer(3, Running, 0, 500_000),
        timer(4, Pending, 0, 0),
    ];
    let Pass { expired, due } = pass(&shown, 100_000);
    assert_eq!(expired, vec![timer(4, Pending, 0, 0)]);
    assert!(due.is_empty());
}

#[test]
fn the_next_wake_is_the_soonest_end_window_or_ended_row_removal() {
    let cases: [(&str, Vec<Timer>, Option<i64>); 6] = [
        ("nothing shown", vec![], None),
        (
            "a running timer wakes at its end",
            vec![timer(1, Running, 0, 50_000)],
            Some(50_000),
        ),
        (
            "a pending timer wakes just after its window",
            vec![timer(1, Pending, 1_000, 999_999)],
            Some(31_001),
        ),
        (
            "an ended row wakes just after its 2 minutes",
            vec![timer(1, Ended, 0, 10_000)],
            Some(130_001),
        ),
        (
            "the soonest of several",
            vec![
                timer(1, Ended, 0, 10_000),
                timer(2, Running, 0, 200_000),
                timer(3, Pending, 5_000, 999_999),
            ],
            Some(35_001),
        ),
        (
            "a cancelled timer never wakes the service",
            vec![timer(1, Cancelled, 0, 0)],
            Some(i64::MAX),
        ),
    ];
    for (case, shown, wake) in cases {
        assert_eq!(next_wake(&shown), wake, "{case}");
    }
}

#[test]
fn a_timer_found_at_start_fires_late_under_an_hour_overdue_and_is_dropped_at_an_hour() {
    let ended_at = 10_000_000;
    let running = timer(1, Running, 0, ended_at);
    let cases = [
        (ended_at, Overdue::Late(Duration::ZERO)),
        (
            ended_at + 40 * 60_000,
            Overdue::Late(Duration::from_secs(40 * 60)),
        ),
        (
            ended_at + 3_599_999,
            Overdue::Late(Duration::from_millis(3_599_999)),
        ),
        (ended_at + 3_600_000, Overdue::Missed),
        (ended_at + 2 * 3_600_000, Overdue::Missed),
    ];
    for (now, expected) in cases {
        assert_eq!(overdue(&running, now), expected, "at {now}");
    }
}
