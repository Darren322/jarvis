use std::collections::HashSet;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use tokio::sync::watch;

use super::{newly_ended, ring_when_timers_end};
use crate::storage::{Timer, TimerId, TimerState};

fn timer(id: i64, state: TimerState) -> Timer {
    Timer {
        id: TimerId(id),
        label: None,
        ends_at_unix_ms: 0,
        created_at_unix_ms: 0,
        state,
    }
}

#[test]
fn only_a_timer_newly_ended_rings() {
    let seen: HashSet<TimerId> = [TimerId(1)].into_iter().collect();
    let shown = [
        timer(1, TimerState::Ended),
        timer(2, TimerState::Ended),
        timer(3, TimerState::Running),
        timer(4, TimerState::Pending),
    ];
    assert_eq!(newly_ended(&seen, &shown), vec![TimerId(2)]);
}

#[tokio::test]
async fn each_timer_rings_once_when_it_ends_and_one_ended_before_never_does() {
    let (sender, receiver) = watch::channel(vec![timer(1, TimerState::Ended)]);
    let rings = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&rings);
    let task = tokio::spawn(async move {
        match ring_when_timers_end(receiver, move || {
            counted.fetch_add(1, Ordering::SeqCst);
        })
        .await {}
    });

    let shown = |states: &[(i64, TimerState)]| {
        states
            .iter()
            .map(|&(id, state)| timer(id, state))
            .collect::<Vec<_>>()
    };
    let wait_for = |count: usize| {
        let rings = Arc::clone(&rings);
        async move {
            while rings.load(Ordering::SeqCst) < count {
                tokio::task::yield_now().await;
            }
        }
    };

    // On this single-threaded test runtime one yield lets the task read the first
    // list and wait for the next, so it starts with only timer 1 ended.
    tokio::task::yield_now().await;
    sender.send_replace(shown(&[(1, TimerState::Ended), (2, TimerState::Running)]));
    sender.send_replace(shown(&[(1, TimerState::Ended), (2, TimerState::Ended)]));
    wait_for(1).await;
    // Still ended in the next list: no second ring.
    sender.send_replace(shown(&[(2, TimerState::Ended), (3, TimerState::Ended)]));
    wait_for(2).await;
    sender.send_replace(shown(&[(3, TimerState::Ended)]));
    for _ in 0..50 {
        tokio::task::yield_now().await;
    }

    assert_eq!(
        rings.load(Ordering::SeqCst),
        2,
        "timers 2 and 3 rang once each"
    );
    task.abort();
}
