//! The service against a real temporary database, on tokio's paused clock.
//!
//! `tokio-rusqlite` runs each call on its own thread, so a paused clock may jump
//! to the next armed sleep during a database call. These tests therefore never
//! sleep: they wait for an event or for the shown timers to change, so the only
//! armed sleep is the service's own deadline. Exact boundaries are tested on the
//! pure planner (`plan_tests.rs`).

use std::{sync::Arc, time::Duration};

use tempfile::TempDir;
use tokio::{sync::mpsc, task::JoinHandle, time::Instant};

use crate::storage::{ConversationArchive, Timer, TimerError, TimerState, TimersRepository};

use super::{
    CancelOutcome, SystemClock, TimerEvent, TimerServiceError, TimersHandle, WallClock, channel,
};

/// 2026-11-22 18:00:00 UTC, the wall-clock time each test starts at.
const START_UNIX_MS: i64 = 1_795_370_400_000;

const MINUTE_MS: i64 = 60_000;

/// A wall clock that moves with tokio's paused clock, so one `advance` moves both.
struct PausedClock {
    base_unix_ms: i64,
    start: Instant,
}

impl PausedClock {
    fn at(base_unix_ms: i64) -> Self {
        Self {
            base_unix_ms,
            start: Instant::now(),
        }
    }
}

impl WallClock for PausedClock {
    fn now_unix_ms(&self) -> i64 {
        let elapsed = i64::try_from(self.start.elapsed().as_millis()).expect("tests run briefly");
        self.base_unix_ms + elapsed
    }
}

/// A temporary database, removed when the test ends, and the service running on it.
struct Fixture {
    directory: TempDir,
    repository: TimersRepository,
    clock: Arc<PausedClock>,
}

/// One running service; it stops when dropped.
struct Service {
    handle: TimersHandle,
    events: mpsc::Receiver<TimerEvent>,
    task: JoinHandle<()>,
}

impl Drop for Service {
    fn drop(&mut self) {
        self.task.abort();
    }
}

struct SharedClock(Arc<PausedClock>);

impl WallClock for SharedClock {
    fn now_unix_ms(&self) -> i64 {
        self.0.now_unix_ms()
    }
}

impl Fixture {
    async fn open() -> Self {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let archive = ConversationArchive::open(directory.path().join("jarvis.sqlite3"))
            .await
            .expect("archive opens");
        Self {
            repository: archive.timers_repository(),
            directory,
            clock: Arc::new(PausedClock::at(START_UNIX_MS)),
        }
    }

    fn now(&self) -> i64 {
        self.clock.now_unix_ms()
    }

    /// Starts a service on the fixture's database and waits until its start-up
    /// pass has published the shown timers.
    async fn start(&self) -> Service {
        let (handle, inbox, events) = channel();
        let repository = self.repository.clone();
        let clock = SharedClock(Arc::clone(&self.clock));
        let task = tokio::spawn(async move { match inbox.run(repository, clock).await {} });
        // A request is answered only after the start-up and the first pass, so its
        // answer means both are done (an empty list never shows as a change).
        let _ = handle
            .cancel_matching(Some("nothing-at-all".to_owned()))
            .await;
        Service {
            handle,
            events,
            task,
        }
    }
}

impl Service {
    /// Proposes a timer ending `minutes` from now and confirms it.
    async fn set(&self, fixture: &Fixture, label: &str, minutes: i64) -> Timer {
        let proposed = self
            .handle
            .propose(Some(label.to_owned()), fixture.now() + minutes * MINUTE_MS)
            .await
            .expect("proposed");
        self.handle.confirm(proposed.id).await.expect("confirmed")
    }

    /// Waits until the shown timers satisfy `done`.
    async fn shown_until(&self, done: impl Fn(&[Timer]) -> bool) -> Vec<Timer> {
        let mut shown = self.handle.watch_shown();
        let timers = shown
            .wait_for(|timers| done(timers))
            .await
            .expect("the service is running");
        timers.clone()
    }

    async fn next_event(&mut self) -> TimerEvent {
        self.events.recv().await.expect("the service is running")
    }
}

fn states(timers: &[Timer]) -> Vec<(Option<&str>, TimerState)> {
    timers
        .iter()
        .map(|timer| (timer.label.as_deref(), timer.state))
        .collect()
}

#[tokio::test(start_paused = true)]
async fn a_confirmed_timer_fires_at_its_end_and_shows_as_ended() {
    let fixture = Fixture::open().await;
    let mut service = fixture.start().await;
    let set = service.set(&fixture, "Time to sleep", 10).await;
    let started = Instant::now();

    let TimerEvent::Ended { timer, late_by } = service.next_event().await else {
        panic!("the timer ends");
    };

    assert_eq!(timer.id, set.id);
    assert_eq!(timer.label.as_deref(), Some("Time to sleep"));
    assert_eq!(late_by, None);
    assert_eq!(started.elapsed(), Duration::from_secs(10 * 60));
    assert_eq!(timer.state, TimerState::Ended);
    // The event comes first; the shown list follows at the end of the same pass.
    let shown = service
        .shown_until(|timers| timers.iter().all(|timer| timer.state == TimerState::Ended))
        .await;
    assert_eq!(
        states(&shown),
        vec![(Some("Time to sleep"), TimerState::Ended)]
    );
}

#[tokio::test(start_paused = true)]
async fn an_ended_row_leaves_the_list_after_two_minutes() {
    let fixture = Fixture::open().await;
    let mut service = fixture.start().await;
    service.set(&fixture, "Tea", 1).await;
    assert!(matches!(
        service.next_event().await,
        TimerEvent::Ended { .. }
    ));
    let ended_at = Instant::now();

    service.shown_until(<[Timer]>::is_empty).await;

    // The row goes on the first pass after 2 minutes (exact edge: plan tests).
    assert_eq!(ended_at.elapsed(), Duration::from_millis(2 * 60_000 + 1));
}

#[tokio::test(start_paused = true)]
async fn confirming_at_30_seconds_sets_the_timer() {
    let fixture = Fixture::open().await;
    let service = fixture.start().await;
    let proposed = service
        .handle
        .propose(None, fixture.now() + 10 * MINUTE_MS)
        .await
        .expect("proposed");
    service.shown_until(|timers| timers.len() == 1).await;

    tokio::time::advance(Duration::from_secs(30)).await;
    let confirmed = service.handle.confirm(proposed.id).await.expect("in time");

    assert_eq!(confirmed.state, TimerState::Running);
}

#[tokio::test(start_paused = true)]
async fn confirming_just_past_30_seconds_is_refused_and_the_timer_is_gone() {
    let fixture = Fixture::open().await;
    let mut service = fixture.start().await;
    let proposed = service
        .handle
        .propose(Some("Late".to_owned()), fixture.now() + 10 * MINUTE_MS)
        .await
        .expect("proposed");
    service.shown_until(|timers| timers.len() == 1).await;

    tokio::time::advance(Duration::from_millis(30_001)).await;
    let refused = service
        .handle
        .confirm(proposed.id)
        .await
        .expect_err("too late");

    // Whether the confirm or the expiry pass came first, the window decides.
    assert!(
        matches!(
            refused,
            TimerServiceError::WindowOver { .. }
                | TimerServiceError::Timer(TimerError::NotInExpectedState { found: None, .. })
        ),
        "{refused:?}"
    );
    let TimerEvent::Expired { timer } = service.next_event().await else {
        panic!("the expiry is reported");
    };
    assert_eq!(timer.id, proposed.id);
    service.shown_until(<[Timer]>::is_empty).await;
    assert!(!service.handle.has_pending());
}

#[tokio::test(start_paused = true)]
async fn an_unconfirmed_timer_expires_by_itself_after_its_window() {
    let fixture = Fixture::open().await;
    let mut service = fixture.start().await;
    service
        .handle
        .propose(None, fixture.now() + 10 * MINUTE_MS)
        .await
        .expect("proposed");
    let proposed_at = Instant::now();
    service.shown_until(|timers| timers.len() == 1).await;
    assert!(service.handle.has_pending());

    assert!(matches!(
        service.next_event().await,
        TimerEvent::Expired { .. }
    ));

    assert_eq!(proposed_at.elapsed(), Duration::from_millis(30_001));
    service.shown_until(<[Timer]>::is_empty).await;
    assert!(!service.handle.has_pending());
}

#[tokio::test(start_paused = true)]
async fn saying_no_drops_a_pending_timer_and_is_refused_for_a_running_one() {
    let fixture = Fixture::open().await;
    let service = fixture.start().await;
    let proposed = service
        .handle
        .propose(None, fixture.now() + MINUTE_MS)
        .await
        .expect("proposed");

    let rejected = service.handle.reject(proposed.id).await.expect("dropped");
    assert_eq!(rejected.state, TimerState::Cancelled);

    let running = service.set(&fixture, "Work", 5).await;
    let refused = service
        .handle
        .reject(running.id)
        .await
        .expect_err("running");
    assert!(
        matches!(
            refused,
            TimerServiceError::NotPending {
                found: Some(TimerState::Running),
                ..
            }
        ),
        "{refused:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn a_sixth_timer_is_refused_until_one_is_cancelled() {
    let fixture = Fixture::open().await;
    let service = fixture.start().await;
    let mut set = Vec::new();
    for minutes in 1..=5 {
        set.push(service.set(&fixture, "Timer", minutes * 10).await);
    }

    let refused = service
        .handle
        .propose(None, fixture.now() + MINUTE_MS)
        .await
        .expect_err("a sixth");
    assert!(
        matches!(
            refused,
            TimerServiceError::Timer(TimerError::LimitReached { limit: 5 })
        ),
        "{refused:?}"
    );

    service.handle.cancel(set[0].id).await.expect("cancelled");
    service
        .handle
        .propose(None, fixture.now() + MINUTE_MS)
        .await
        .expect("room again");
}

#[tokio::test(start_paused = true)]
async fn a_spoken_cancel_takes_the_timer_it_names_and_lists_the_rest_otherwise() {
    let fixture = Fixture::open().await;
    let service = fixture.start().await;
    service.set(&fixture, "Time to sleep", 60).await;
    service.set(&fixture, "Time to do work", 30).await;

    let several = service.handle.cancel_matching(None).await.expect("asked");
    let CancelOutcome::Several(matched) = several else {
        panic!("two timers match no words: {several:?}");
    };
    assert_eq!(matched.len(), 2);

    let none = service
        .handle
        .cancel_matching(Some("oven".to_owned()))
        .await
        .expect("asked");
    assert!(
        matches!(none, CancelOutcome::NoMatch(ref left) if left.len() == 2),
        "{none:?}"
    );

    let one = service
        .handle
        .cancel_matching(Some("the sleep one".to_owned()))
        .await
        .expect("cancelled");
    let CancelOutcome::Cancelled(cancelled) = one else {
        panic!("the sleep one is cancelled: {one:?}");
    };
    assert_eq!(cancelled.label.as_deref(), Some("Time to sleep"));
    let shown = service.shown_until(|timers| timers.len() == 1).await;
    assert_eq!(
        states(&shown),
        vec![(Some("Time to do work"), TimerState::Running)]
    );
}

#[tokio::test(start_paused = true)]
async fn two_timers_ending_together_each_fire_once() {
    let fixture = Fixture::open().await;
    let mut service = fixture.start().await;
    let ends_at = fixture.now() + 5 * MINUTE_MS;
    for label in ["One", "Two"] {
        let proposed = service
            .handle
            .propose(Some(label.to_owned()), ends_at)
            .await
            .expect("proposed");
        service
            .handle
            .confirm(proposed.id)
            .await
            .expect("confirmed");
    }

    let mut ended = Vec::new();
    for _ in 0..2 {
        let TimerEvent::Ended { timer, .. } = service.next_event().await else {
            panic!("both end");
        };
        ended.push(timer.label.expect("labelled"));
    }

    ended.sort();
    assert_eq!(ended, ["One", "Two"]);
    service
        .shown_until(|timers| timers.iter().all(|timer| timer.state == TimerState::Ended))
        .await;
    assert!(service.events.try_recv().is_err(), "no third event");
}

#[tokio::test(start_paused = true)]
async fn a_cancel_at_the_moment_a_timer_ends_leaves_one_consistent_result() {
    let fixture = Fixture::open().await;
    let mut service = fixture.start().await;
    let set = service.set(&fixture, "Race", 1).await;
    service.shown_until(|timers| timers.len() == 1).await;

    tokio::time::advance(Duration::from_secs(60)).await;
    let cancelled = service.handle.cancel(set.id).await.expect("cancelled");

    service.shown_until(<[Timer]>::is_empty).await;
    let mut ended_events = 0;
    while let Ok(event) = service.events.try_recv() {
        assert!(matches!(event, TimerEvent::Ended { .. }), "{event:?}");
        ended_events += 1;
    }
    // Either the cancel won (no reminder) or the timer had ended and the cancel
    // removed its row (one reminder); never both, never twice.
    assert!(ended_events <= 1);
    assert_eq!(cancelled.state, TimerState::Cancelled);
}

#[tokio::test(start_paused = true)]
async fn timers_survive_a_restart_and_still_fire() {
    let fixture = Fixture::open().await;
    let first = fixture.start().await;
    let set = first.set(&fixture, "Time to sleep", 10).await;
    drop(first);

    let mut second = fixture.start().await;
    let TimerEvent::Ended { timer, late_by } = second.next_event().await else {
        panic!("the timer still ends");
    };

    assert_eq!(timer.id, set.id);
    assert_eq!(late_by, None);
}

#[tokio::test(start_paused = true)]
async fn at_start_a_recent_missed_timer_fires_late_an_old_one_is_dropped_and_pending_ones_go() {
    let fixture = Fixture::open().await;
    let now = fixture.now();
    for (label, ended_minutes_ago) in [("Recent", 40), ("Old", 120)] {
        let proposed = fixture
            .repository
            .insert_pending(
                Some(label),
                now - ended_minutes_ago * MINUTE_MS,
                now - 200 * MINUTE_MS,
                5,
            )
            .await
            .expect("inserted");
        fixture
            .repository
            .confirm(proposed.id)
            .await
            .expect("running");
    }
    fixture
        .repository
        .insert_pending(Some("Left pending"), now + MINUTE_MS, now - 1, 5)
        .await
        .expect("inserted");

    let mut service = fixture.start().await;

    let mut late = None;
    let mut missed = None;
    for _ in 0..2 {
        match service.next_event().await {
            TimerEvent::Ended {
                timer,
                late_by: Some(late_by),
            } => late = Some((timer.label, late_by)),
            TimerEvent::Missed { timer } => missed = Some(timer.label),
            other => panic!("unexpected {other:?}"),
        }
    }
    assert_eq!(
        late,
        Some((Some("Recent".to_owned()), Duration::from_secs(40 * 60)))
    );
    assert_eq!(missed, Some(Some("Old".to_owned())));
    // Both ended long ago, so neither is shown; the leftover pending one is gone
    // without a word (D22).
    assert!(service.handle.shown().is_empty());
    assert!(service.events.try_recv().is_err());
}

#[tokio::test(start_paused = true)]
async fn nobody_listening_for_events_never_stops_a_timer_from_firing() {
    let fixture = Fixture::open().await;
    let mut service = fixture.start().await;
    service.set(&fixture, "Unheard", 3).await;
    service.events.close();

    let shown = service
        .shown_until(|timers| timers.iter().any(|timer| timer.state == TimerState::Ended))
        .await;

    assert_eq!(states(&shown), vec![(Some("Unheard"), TimerState::Ended)]);
}

#[tokio::test(start_paused = true)]
async fn a_pass_that_cannot_read_the_timers_is_reported_and_tried_again() {
    let fixture = Fixture::open().await;
    let mut service = fixture.start().await;
    let raw = rusqlite::Connection::open(fixture.directory.path().join("jarvis.sqlite3"))
        .expect("a second connection");
    raw.execute_batch("ALTER TABLE timers RENAME TO timers_away")
        .expect("table renamed");

    let TimerEvent::Failed(error) = service.next_event().await else {
        panic!("the failure is reported");
    };
    assert!(error.to_string().contains("no such table"), "{error}");

    raw.execute_batch("ALTER TABLE timers_away RENAME TO timers")
        .expect("table back");
    service.set(&fixture, "After the failure", 1).await;
    assert!(matches!(
        service.next_event().await,
        TimerEvent::Ended { .. }
    ));
}

#[tokio::test]
async fn the_system_clock_reads_the_real_time() {
    // 2026-01-01, well before any machine running these tests.
    assert!(SystemClock.now_unix_ms() > 1_767_225_600_000);
}
