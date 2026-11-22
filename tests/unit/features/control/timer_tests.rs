//! Answering a proposed timer against a real timer service on a temporary
//! database, and the fixed lines at fixed instants. The service's clock only
//! moves when a test moves it, and nothing here sleeps.

use std::sync::{
    Arc,
    atomic::{AtomicI64, Ordering},
};
use std::time::Duration;

use jiff::Zoned;
use tempfile::TempDir;
use tokio::task::JoinHandle;

use super::{
    AnswerOutcome, REJECTED_LINE, TimerAnswer, answer_timer, event_notice, expired_line,
    proposal_line, set_line,
};
use crate::features::control::Notice;
use crate::features::timers::{TimerEvent, TimersHandle, WallClock, channel};
use crate::storage::{ConversationArchive, Timer, TimerId, TimerState};

/// A Sunday evening in Singapore.
const NOW: &str = "2026-11-22T18:00:00[Asia/Singapore]";
const MINUTE_MS: i64 = 60_000;

fn now() -> Zoned {
    NOW.parse().expect("the test instant is valid")
}

fn now_ms() -> i64 {
    now().timestamp().as_millisecond()
}

/// A wall clock that moves only when a test moves it.
#[derive(Clone)]
struct SetClock(Arc<AtomicI64>);

impl WallClock for SetClock {
    fn now_unix_ms(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

struct Service {
    handle: TimersHandle,
    clock: Arc<AtomicI64>,
    task: JoinHandle<()>,
    _directory: TempDir,
}

impl Drop for Service {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Service {
    async fn start() -> Self {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let archive = ConversationArchive::open(directory.path().join("jarvis.sqlite3"))
            .await
            .expect("archive opens");
        let repository = archive.timers_repository();
        let clock = Arc::new(AtomicI64::new(now_ms()));
        let (handle, inbox, _events) = channel();
        let service_clock = SetClock(Arc::clone(&clock));
        let task = tokio::spawn(async move { match inbox.run(repository, service_clock).await {} });
        Self {
            handle,
            clock,
            task,
            _directory: directory,
        }
    }

    /// Proposes a timer and waits until the shown timers carry it, as `parse`'s
    /// caller sees them.
    async fn propose(&self, label: Option<&str>, minutes: i64) -> Timer {
        let timer = self
            .handle
            .propose(label.map(str::to_owned), now_ms() + minutes * MINUTE_MS)
            .await
            .expect("proposed");
        self.handle
            .watch_shown()
            .wait_for(|timers| timers.iter().any(|shown| shown.id == timer.id))
            .await
            .expect("the service is running");
        timer
    }

    async fn state_of(&self, id: TimerId) -> Option<TimerState> {
        let mut shown = self.handle.watch_shown();
        let timers = shown
            .wait_for(|timers| {
                timers
                    .iter()
                    .all(|timer| timer.id != id || timer.state != TimerState::Pending)
            })
            .await
            .expect("the service is running");
        timers
            .iter()
            .find(|timer| timer.id == id)
            .map(|timer| timer.state)
    }
}

fn timer(label: Option<&str>, ends_in_minutes: i64) -> Timer {
    Timer {
        id: TimerId(1),
        label: label.map(str::to_owned),
        ends_at_unix_ms: now_ms() + ends_in_minutes * MINUTE_MS,
        created_at_unix_ms: now_ms(),
        state: TimerState::Pending,
    }
}

#[test]
fn the_fixed_lines_name_the_timer_and_its_time() {
    let sleep = timer(Some("Time to sleep"), 5 * 60);
    let plain = timer(None, 10);
    let cases = [
        (
            proposal_line(&sleep, &now()),
            "Set 'Time to sleep' for 11:00 pm today? Say yes or tap Set.",
        ),
        (
            proposal_line(&plain, &now()),
            "Set the timer for 6:10 pm today? Say yes or tap Set.",
        ),
        (
            set_line(&sleep, &now()),
            "Set. 'Time to sleep' ends at 11:00 pm today.",
        ),
        (
            set_line(&plain, &now()),
            "Set. The timer ends at 6:10 pm today.",
        ),
        (
            expired_line(&sleep),
            "No yes came, so I dropped 'Time to sleep'.",
        ),
        (expired_line(&plain), "No yes came, so I dropped the timer."),
    ];
    for (line, expected) in cases {
        assert_eq!(line, expected);
    }
}

#[test]
fn each_timer_event_has_its_line() {
    let sleep = timer(Some("Time to sleep"), 0);
    let cases = [
        (
            TimerEvent::Ended {
                timer: sleep.clone(),
                late_by: None,
            },
            "'Time to sleep' has ended.",
        ),
        (
            TimerEvent::Ended {
                timer: timer(None, 0),
                late_by: Some(Duration::from_secs(40 * 60 + 59)),
            },
            "The timer ended 40 minutes ago, while I was off.",
        ),
        (
            TimerEvent::Ended {
                timer: sleep.clone(),
                late_by: Some(Duration::from_secs(59)),
            },
            "'Time to sleep' ended less than a minute ago, while I was off.",
        ),
        (
            TimerEvent::Missed {
                timer: sleep.clone(),
            },
            "'Time to sleep' ended over an hour ago, while I was off.",
        ),
        (
            TimerEvent::Expired { timer: sleep },
            "No yes came, so I dropped 'Time to sleep'.",
        ),
    ];
    for (event, expected) in cases {
        assert_eq!(event_notice(&event), Notice::Info(expected.to_owned()));
    }
}

#[tokio::test]
async fn yes_sets_the_pending_timer_and_says_so() {
    let service = Service::start().await;
    let proposed = service.propose(Some("Time to sleep"), 60).await;

    let outcome = answer_timer(&service.handle, TimerAnswer::Yes).await;

    let AnswerOutcome::Answered(Notice::Info(line)) = outcome else {
        panic!("answered: {outcome:?}");
    };
    assert!(line.starts_with("Set. 'Time to sleep' ends at "), "{line}");
    assert_eq!(
        service.state_of(proposed.id).await,
        Some(TimerState::Running)
    );
}

#[tokio::test]
async fn no_drops_the_pending_timer_and_says_so() {
    let service = Service::start().await;
    let proposed = service.propose(None, 10).await;

    let outcome = answer_timer(&service.handle, TimerAnswer::No).await;

    assert_eq!(
        outcome,
        AnswerOutcome::Answered(Notice::Info(REJECTED_LINE.to_owned()))
    );
    assert_eq!(service.state_of(proposed.id).await, None);
}

#[tokio::test]
async fn with_no_timer_waiting_yes_and_no_are_for_the_model() {
    let service = Service::start().await;
    for answer in [TimerAnswer::Yes, TimerAnswer::No] {
        assert_eq!(
            answer_timer(&service.handle, answer).await,
            AnswerOutcome::NotForATimer
        );
    }
}

#[tokio::test]
async fn yes_at_30_seconds_sets_the_timer_and_just_after_is_for_the_model() {
    for (waited_ms, sets_it) in [(30_000, true), (30_001, false)] {
        let service = Service::start().await;
        let proposed = service.propose(None, 10).await;
        service.clock.fetch_add(waited_ms, Ordering::SeqCst);

        let outcome = answer_timer(&service.handle, TimerAnswer::Yes).await;

        assert_eq!(
            matches!(outcome, AnswerOutcome::Answered(_)),
            sets_it,
            "after {waited_ms} ms: {outcome:?}"
        );
        assert!(
            service.handle.confirm(proposed.id).await.is_err(),
            "after {waited_ms} ms the timer is no longer pending"
        );
    }
}
