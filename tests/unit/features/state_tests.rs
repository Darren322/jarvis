use super::*;
use std::time::Duration;
use tokio::time::timeout;

const WAIT: Duration = Duration::from_secs(1);

#[tokio::test(start_paused = true)]
async fn every_change_reaches_an_observer_in_order() {
    let feed = StateFeed::new();
    let mut observer = feed.observe();
    assert_eq!(observer.current(), AssistantState::Idle);

    // Published back to back, with no await between them.
    let changes = [
        AssistantState::Listening,
        AssistantState::Transcribing,
        AssistantState::Thinking,
        AssistantState::Speaking,
        AssistantState::Idle,
    ];
    for state in changes {
        feed.publish(state);
    }
    for state in changes {
        assert_eq!(observer.next().await, state);
        assert_eq!(observer.current(), state);
    }
}

#[tokio::test(start_paused = true)]
async fn a_repeated_state_is_not_a_change() {
    let feed = StateFeed::new();
    let mut observer = feed.observe();
    feed.publish(AssistantState::Idle);
    feed.publish(AssistantState::Thinking);
    feed.publish(AssistantState::Thinking);
    assert_eq!(observer.next().await, AssistantState::Thinking);
    assert!(timeout(WAIT, observer.next()).await.is_err());
}

#[tokio::test(start_paused = true)]
async fn a_new_observer_starts_from_the_current_state() {
    let feed = StateFeed::new();
    feed.publish(AssistantState::Speaking);
    let mut observer = feed.observe();
    assert_eq!(observer.current(), AssistantState::Speaking);
    feed.publish(AssistantState::Idle);
    assert_eq!(observer.next().await, AssistantState::Idle);
}

#[tokio::test(start_paused = true)]
async fn an_observer_that_falls_behind_skips_to_what_is_still_queued() {
    let feed = StateFeed::new();
    let mut observer = feed.observe();
    // The last accepted backlog is the queue length; one more loses the oldest.
    let cycle = [AssistantState::Thinking, AssistantState::Idle];
    for state in cycle.into_iter().cycle().take(STATE_CHANGE_QUEUE + 1) {
        feed.publish(state);
    }
    // 17 changes ended on Thinking; the observer still arrives there.
    let mut last = observer.current();
    while let Ok(state) = timeout(WAIT, observer.next()).await {
        last = state;
    }
    assert_eq!(last, AssistantState::Thinking);
}

#[tokio::test(start_paused = true)]
async fn an_observer_outliving_the_feed_just_waits() {
    let feed = StateFeed::new();
    let mut observer = feed.observe();
    drop(feed);
    assert!(timeout(WAIT, observer.next()).await.is_err());
}
