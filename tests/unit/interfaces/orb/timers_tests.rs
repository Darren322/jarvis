use std::time::Duration;

use orb_protocol::{MAX_TIMER_LABEL_CHARS, TimerState as Wire};
use tokio::time::timeout;

use super::*;

/// The largest id storage can hold, as the wire's `u64`.
const LARGEST_STORED_ID: u64 = i64::MAX as u64;

fn stored(id: i64, label: Option<&str>, ends_at: i64, state: TimerState) -> Timer {
    Timer {
        id: TimerId(id),
        label: label.map(str::to_owned),
        ends_at_unix_ms: ends_at,
        created_at_unix_ms: 0,
        state,
    }
}

fn wire(id: u64, label: Option<&str>, ends_at: u64, state: Wire) -> orb_protocol::Timer {
    orb_protocol::Timer::new(id, label.map(str::to_owned), ends_at, state)
        .expect("the label is within the limit")
}

fn listed(message: ToOrb) -> Vec<orb_protocol::Timer> {
    let ToOrb::Timers { timers } = message else {
        panic!("not a timers message: {message:?}");
    };
    timers
}

fn ids(timers: &[orb_protocol::Timer]) -> Vec<u64> {
    timers.iter().map(orb_protocol::Timer::id).collect()
}

#[test]
fn each_state_is_listed_with_its_label_and_time_and_what_does_not_fit_is_left_out() {
    let long_label = "a".repeat(MAX_TIMER_LABEL_CHARS);
    let too_long_label = format!("{long_label}a");
    let cases = [
        (
            stored(1, None, 1_000, TimerState::Pending),
            Some(wire(1, None, 1_000, Wire::Pending)),
        ),
        (
            stored(2, Some("Time to sleep"), 2_000, TimerState::Running),
            Some(wire(2, Some("Time to sleep"), 2_000, Wire::Running)),
        ),
        (
            stored(3, Some("Pasta"), 3_000, TimerState::Ended),
            Some(wire(3, Some("Pasta"), 3_000, Wire::Ended)),
        ),
        // The widest id and time that fit, and the smallest.
        (
            stored(i64::MAX, None, i64::MAX, TimerState::Running),
            Some(wire(
                LARGEST_STORED_ID,
                None,
                LARGEST_STORED_ID,
                Wire::Running,
            )),
        ),
        (
            stored(0, None, 0, TimerState::Running),
            Some(wire(0, None, 0, Wire::Running)),
        ),
        // The last label that fits, and the first that does not.
        (
            stored(4, Some(&long_label), 4_000, TimerState::Running),
            Some(wire(4, Some(&long_label), 4_000, Wire::Running)),
        ),
        (
            stored(5, Some(&too_long_label), 5_000, TimerState::Running),
            None,
        ),
        // A cancelled timer is never listed.
        (stored(6, None, 6_000, TimerState::Cancelled), None),
        // An id or an end time that is not a `u64` is left out.
        (stored(-1, None, 7_000, TimerState::Running), None),
        (stored(8, None, -1, TimerState::Running), None),
    ];
    for (timer, expected) in cases {
        assert_eq!(
            listed(timers_message(std::slice::from_ref(&timer))),
            expected.into_iter().collect::<Vec<_>>(),
            "{timer:?}"
        );
    }
}

#[test]
fn a_timer_that_cannot_be_sent_leaves_the_rest_of_the_list_alone() {
    let message = timers_message(&[
        stored(1, None, 1_000, TimerState::Running),
        stored(-2, None, 2_000, TimerState::Running),
        stored(3, None, 3_000, TimerState::Pending),
    ]);
    assert_eq!(ids(&listed(message)), [1, 3]);
    assert_eq!(listed(timers_message(&[])), Vec::new());
}

/// A task that ended by itself within a second, as opposed to being cut short.
async fn ended_by_itself(task: tokio::task::JoinHandle<()>) -> bool {
    matches!(timeout(Duration::from_secs(1), task).await, Ok(Ok(())))
}

#[tokio::test(start_paused = true)]
async fn the_list_is_sent_once_at_the_start_and_once_for_each_change() {
    let (shown, shown_receiver) = watch::channel(vec![stored(1, None, 1_000, TimerState::Running)]);
    let (to_orb, mut sent) = mpsc::channel(8);
    let task = tokio::spawn(publish_timers(shown_receiver, to_orb));

    let first = listed(sent.recv().await.expect("sent at the start"));
    assert_eq!(ids(&first), [1]);
    // Nothing changed: nothing more is sent.
    assert!(timeout(Duration::from_secs(1), sent.recv()).await.is_err());

    shown.send_replace(vec![
        stored(1, None, 1_000, TimerState::Ended),
        stored(2, Some("Pasta"), 2_000, TimerState::Pending),
    ]);
    let second = listed(sent.recv().await.expect("sent for the change"));
    assert_eq!(ids(&second), [1, 2]);
    assert!(timeout(Duration::from_secs(1), sent.recv()).await.is_err());

    shown.send_replace(Vec::new());
    assert_eq!(listed(sent.recv().await.expect("the clearing")), Vec::new());

    // With the timers gone there is nothing more to follow.
    drop(shown);
    assert!(ended_by_itself(task).await);
    assert!(sent.recv().await.is_none());
}

#[tokio::test(start_paused = true)]
async fn the_list_stops_being_followed_when_the_orb_server_is_gone() {
    let (_shown, shown_receiver) = watch::channel(Vec::new());
    let (to_orb, sent) = mpsc::channel(8);
    drop(sent);
    let task = tokio::spawn(publish_timers(shown_receiver, to_orb));
    assert!(ended_by_itself(task).await);
}

#[tokio::test(start_paused = true)]
async fn taps_come_in_order_and_one_whose_id_cannot_be_stored_is_skipped() {
    let (messages, receiver) = mpsc::channel(8);
    let mut taps = TimerTaps::new(receiver);
    for message in [
        FromOrb::ConfirmTimer { id: 1 },
        FromOrb::RejectTimer { id: 2 },
        // One past the largest stored id, and the largest.
        FromOrb::CancelTimer {
            id: LARGEST_STORED_ID + 1,
        },
        FromOrb::CancelTimer {
            id: LARGEST_STORED_ID,
        },
        FromOrb::Tap {},
        FromOrb::PanelClosed {},
        FromOrb::CancelTimer { id: 4 },
    ] {
        messages.send(message).await.expect("room");
    }
    drop(messages);

    let mut heard = Vec::new();
    while let Some(tap) = taps.next().await {
        heard.push(tap);
    }
    assert_eq!(
        heard,
        [
            TimerTap::Confirm(TimerId(1)),
            TimerTap::Reject(TimerId(2)),
            TimerTap::Cancel(TimerId(i64::MAX)),
            TimerTap::Cancel(TimerId(4)),
        ]
    );
}
