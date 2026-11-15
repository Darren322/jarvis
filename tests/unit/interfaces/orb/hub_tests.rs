use std::time::Duration;

use orb_protocol::{Level, LevelSource};
use tokio::time::timeout;

use super::*;

fn state(state: OrbState) -> ToOrb {
    ToOrb::State { state }
}

/// A level whose value says which one it is, so order and loss can be seen.
fn level(number: usize) -> ToOrb {
    ToOrb::Level {
        level: Level::new(number as f32 / 100.0).unwrap(),
        source: LevelSource::Mic,
    }
}

/// Alternating states, so each one differs from the one before.
fn state_number(number: usize) -> ToOrb {
    state(if number.is_multiple_of(2) {
        OrbState::Thinking
    } else {
        OrbState::Speaking
    })
}

/// Nothing is waiting for the orb: the next read would block.
async fn is_quiet(outbox: &mut Outbox) -> bool {
    timeout(Duration::from_millis(1), outbox.next())
        .await
        .is_err()
}

#[tokio::test(start_paused = true)]
async fn a_new_outbox_starts_with_the_state_as_it_is_and_then_follows_the_changes() {
    let hub = Hub::new(OrbState::Idle);
    hub.publish(state(OrbState::Thinking));

    let mut outbox = hub.subscribe();
    assert_eq!(outbox.next().await, Ok(state(OrbState::Thinking)));
    assert!(is_quiet(&mut outbox).await);

    hub.publish(state(OrbState::Speaking));
    hub.publish(state(OrbState::Idle));
    assert_eq!(outbox.next().await, Ok(state(OrbState::Speaking)));
    assert_eq!(outbox.next().await, Ok(state(OrbState::Idle)));
}

#[tokio::test(start_paused = true)]
async fn an_orb_may_be_this_many_events_behind_and_one_more_cuts_it_off() {
    // (events published while the orb does not read, whether all arrive)
    let cases = [(EVENT_QUEUE, true), (EVENT_QUEUE + 1, false)];
    for (published, all_arrive) in cases {
        let hub = Hub::new(OrbState::Idle);
        let mut outbox = hub.subscribe();
        assert_eq!(outbox.next().await, Ok(state(OrbState::Idle)));
        for number in 0..published {
            hub.publish(state_number(number));
        }
        if all_arrive {
            for number in 0..published {
                assert_eq!(outbox.next().await, Ok(state_number(number)), "{published}");
            }
            assert!(is_quiet(&mut outbox).await, "{published}");
        } else {
            // Nothing is delivered out of order or with a hole in it.
            assert_eq!(outbox.next().await, Err(Gone), "{published}");
        }
    }
}

#[tokio::test(start_paused = true)]
async fn an_orb_that_is_behind_keeps_only_the_newest_levels() {
    // (levels published while the orb does not read, the first one that survives)
    let cases = [(LEVEL_QUEUE, 0), (LEVEL_QUEUE + 1, 1)];
    for (published, first_kept) in cases {
        let hub = Hub::new(OrbState::Idle);
        let mut outbox = hub.subscribe();
        assert_eq!(outbox.next().await, Ok(state(OrbState::Idle)));
        for number in 0..published {
            hub.publish(level(number));
        }
        for number in first_kept..published {
            assert_eq!(outbox.next().await, Ok(level(number)), "{published}");
        }
        assert!(is_quiet(&mut outbox).await, "{published}");
    }
}

#[tokio::test(start_paused = true)]
async fn a_full_level_queue_does_not_hold_up_the_next_state_change() {
    let hub = Hub::new(OrbState::Idle);
    let mut outbox = hub.subscribe();
    assert_eq!(outbox.next().await, Ok(state(OrbState::Idle)));
    for number in 0..=LEVEL_QUEUE {
        hub.publish(level(number));
    }
    hub.publish(state(OrbState::Thinking));

    assert_eq!(outbox.next().await, Ok(state(OrbState::Thinking)));
    // The levels were measured before the change: none follows it.
    assert!(is_quiet(&mut outbox).await);
    // Levels after the change are sent as usual.
    hub.publish(level(50));
    assert_eq!(outbox.next().await, Ok(level(50)));
}

#[tokio::test(start_paused = true)]
async fn each_orb_has_its_own_queues() {
    let hub = Hub::new(OrbState::Idle);
    let mut behind = hub.subscribe();
    let mut keeping_up = hub.subscribe();
    assert_eq!(behind.next().await, Ok(state(OrbState::Idle)));
    assert_eq!(keeping_up.next().await, Ok(state(OrbState::Idle)));

    for number in 0..=EVENT_QUEUE {
        hub.publish(state_number(number));
        assert_eq!(keeping_up.next().await, Ok(state_number(number)));
    }
    assert_eq!(behind.next().await, Err(Gone));
}

#[tokio::test(start_paused = true)]
async fn orbs_are_let_go_when_the_hub_is() {
    let hub = Hub::new(OrbState::Idle);
    let handle = hub.handle();
    let mut outbox = handle.subscribe().unwrap();
    assert_eq!(outbox.next().await, Ok(state(OrbState::Idle)));

    drop(hub);
    assert_eq!(outbox.next().await, Err(Gone));
    assert!(handle.subscribe().is_none());
}
