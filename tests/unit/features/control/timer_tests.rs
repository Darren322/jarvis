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
    AnswerOutcome, REJECTED_LINE, TimerAnswer, TimerReaction, TimerTap, answer_tap, answer_timer,
    cancelled_line, expired_line, mentions_timer, proposal_line, react_to, set_line,
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
fn each_timer_event_has_its_reaction() {
    // Set at 6 pm (`NOW`), ended five hours later.
    let sleep = timer(Some("Time to sleep"), 5 * 60);
    let plain = timer(None, 10);
    let cases = [
        (
            TimerEvent::Ended {
                timer: sleep.clone(),
                late_by: None,
            },
            TimerReaction::Remind(
                "My timer 'Time to sleep', set at 6:00 pm, has just ended. Remind me in a sentence or two."
                    .to_owned(),
            ),
        ),
        (
            TimerEvent::Ended {
                timer: plain.clone(),
                late_by: Some(Duration::from_secs(40 * 60 + 59)),
            },
            TimerReaction::Remind(
                "My timer, set at 6:00 pm, ended 40 minutes ago, while you were off. Remind me in a sentence or two."
                    .to_owned(),
            ),
        ),
        (
            TimerEvent::Ended {
                timer: sleep.clone(),
                late_by: Some(Duration::from_secs(59)),
            },
            TimerReaction::Remind(
                "My timer 'Time to sleep', set at 6:00 pm, ended less than a minute ago, while you were off. Remind me in a sentence or two."
                    .to_owned(),
            ),
        ),
        (
            TimerEvent::Missed {
                timer: sleep.clone(),
            },
            TimerReaction::Show(Notice::Info(
                "'Time to sleep' ended over an hour ago, while I was off, so I let it go.".to_owned(),
            )),
        ),
        (
            TimerEvent::Expired { timer: sleep },
            TimerReaction::Say(Notice::Info(
                "No yes came, so I dropped 'Time to sleep'.".to_owned(),
            )),
        ),
    ];
    for (event, expected) in cases {
        assert_eq!(react_to(&event, &now()), expected);
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

/// A proposed timer, confirmed through the service.
async fn running(service: &Service, label: Option<&str>, minutes: i64) -> Timer {
    let proposed = service.propose(label, minutes).await;
    service
        .handle
        .confirm(proposed.id)
        .await
        .expect("confirmed in its window")
}

/// Moves the clock past the end of `timer` and waits until the service has ended
/// it. Any request wakes the service for the pass that ends it; an unknown id is
/// refused and changes nothing.
async fn let_end(service: &Service, timer: &Timer) {
    service
        .clock
        .store(timer.ends_at_unix_ms + 1, Ordering::SeqCst);
    let _ = service.handle.cancel(TimerId(i64::MAX)).await;
    service
        .handle
        .watch_shown()
        .wait_for(|timers| {
            timers
                .iter()
                .any(|shown| shown.id == timer.id && shown.state == TimerState::Ended)
        })
        .await
        .expect("the service is running");
}

/// Waits until the shown timers no longer carry `id`, then says whether any timer
/// is still running.
async fn still_running_once_gone(service: &Service, id: TimerId) -> bool {
    let shown = service
        .handle
        .watch_shown()
        .wait_for(|timers| timers.iter().all(|timer| timer.id != id))
        .await
        .expect("the service is running")
        .clone();
    shown.iter().any(|timer| timer.state == TimerState::Running)
}

#[tokio::test]
async fn a_tap_on_set_sets_the_pending_timer_and_says_so() {
    let service = Service::start().await;
    let proposed = service.propose(Some("Time to sleep"), 60).await;

    let notice = answer_tap(&service.handle, TimerTap::Confirm(proposed.id)).await;

    let Some(Notice::Info(line)) = notice else {
        panic!("a line: {notice:?}");
    };
    assert!(line.starts_with("Set. 'Time to sleep' ends at "), "{line}");
    assert_eq!(
        service.state_of(proposed.id).await,
        Some(TimerState::Running)
    );
}

#[tokio::test]
async fn a_tap_on_dont_drops_the_pending_timer_and_says_so() {
    let service = Service::start().await;
    let proposed = service.propose(None, 10).await;

    let notice = answer_tap(&service.handle, TimerTap::Reject(proposed.id)).await;

    assert_eq!(notice, Some(Notice::Info(REJECTED_LINE.to_owned())));
    assert_eq!(service.state_of(proposed.id).await, None);
}

#[tokio::test]
async fn a_tap_on_set_at_30_seconds_sets_the_timer_and_just_after_sets_nothing() {
    for (waited_ms, sets_it) in [(30_000, true), (30_001, false)] {
        let service = Service::start().await;
        let proposed = service.propose(None, 10).await;
        service.clock.fetch_add(waited_ms, Ordering::SeqCst);

        let notice = answer_tap(&service.handle, TimerTap::Confirm(proposed.id)).await;

        assert_eq!(
            notice.is_some(),
            sets_it,
            "after {waited_ms} ms: {notice:?}"
        );
        let state = service.state_of(proposed.id).await;
        assert_eq!(
            state,
            sets_it.then_some(TimerState::Running),
            "after {waited_ms} ms"
        );
    }
}

#[tokio::test]
async fn cancelling_a_running_timer_from_the_screen_removes_it_and_says_so() {
    let cases = [
        (Some("Time to sleep"), "Cancelled 'Time to sleep'."),
        (None, "Cancelled the timer."),
    ];
    for (label, expected) in cases {
        let service = Service::start().await;
        let timer = running(&service, label, 60).await;

        let notice = answer_tap(&service.handle, TimerTap::Cancel(timer.id)).await;

        assert_eq!(notice, Some(Notice::Info(expected.to_owned())));
        assert!(
            !still_running_once_gone(&service, timer.id).await,
            "the only timer is gone"
        );
    }
}

#[tokio::test]
async fn cancelling_a_pending_timer_from_the_screen_says_so() {
    let service = Service::start().await;
    let proposed = service.propose(Some("Pasta"), 10).await;

    let notice = answer_tap(&service.handle, TimerTap::Cancel(proposed.id)).await;

    assert_eq!(notice, Some(Notice::Info("Cancelled 'Pasta'.".to_owned())));
    assert_eq!(service.state_of(proposed.id).await, None);
}

#[tokio::test]
async fn tapping_an_ended_timer_removes_its_row_and_says_nothing() {
    let service = Service::start().await;
    let timer = running(&service, Some("Pasta"), 5).await;
    let other = running(&service, Some("Time to sleep"), 600).await;
    let_end(&service, &timer).await;

    let notice = answer_tap(&service.handle, TimerTap::Cancel(timer.id)).await;

    assert_eq!(notice, None);
    // The row is gone, and the other timer is untouched.
    assert!(
        still_running_once_gone(&service, timer.id).await,
        "the other one runs"
    );
    assert_eq!(service.state_of(other.id).await, Some(TimerState::Running));
}

#[tokio::test]
async fn a_tap_that_the_timers_do_not_allow_changes_nothing_and_says_nothing() {
    let service = Service::start().await;
    let timer = running(&service, Some("Time to sleep"), 60).await;
    let unknown = TimerId(i64::MAX - 1);
    let cancelled = running(&service, None, 30).await;
    answer_tap(&service.handle, TimerTap::Cancel(cancelled.id)).await;
    // Proposed last: a new proposal replaces a pending one (D23).
    let waiting = service.propose(Some("Pasta"), 10).await;

    let taps = [
        // An id that never was.
        TimerTap::Confirm(unknown),
        TimerTap::Reject(unknown),
        TimerTap::Cancel(unknown),
        // An id already cancelled.
        TimerTap::Confirm(cancelled.id),
        TimerTap::Reject(cancelled.id),
        TimerTap::Cancel(cancelled.id),
        // A timer that is already running is not waiting for an answer.
        TimerTap::Confirm(timer.id),
        TimerTap::Reject(timer.id),
    ];
    for tap in taps {
        assert_eq!(answer_tap(&service.handle, tap).await, None, "{tap:?}");
    }

    // Still there, still as they were.
    assert_eq!(service.state_of(timer.id).await, Some(TimerState::Running));
    let still_waiting = service
        .handle
        .shown()
        .into_iter()
        .find(|shown| shown.id == waiting.id)
        .map(|shown| shown.state);
    assert_eq!(still_waiting, Some(TimerState::Pending));
}

#[tokio::test]
async fn a_tap_while_the_timers_are_not_running_is_a_warning() {
    let (handle, inbox, _events) = channel();
    drop(inbox);
    for tap in [TimerTap::Confirm(TimerId(1)), TimerTap::Reject(TimerId(1))] {
        let notice = answer_tap(&handle, tap).await;
        assert!(
            matches!(&notice, Some(Notice::Warning(text)) if text.contains("not running")),
            "{tap:?}: {notice:?}"
        );
    }
}

#[test]
fn cancelling_is_said_with_the_timers_name() {
    let cases = [
        (Some("Time to sleep"), "Cancelled 'Time to sleep'."),
        (None, "Cancelled the timer."),
    ];
    for (label, expected) in cases {
        assert_eq!(cancelled_line(&timer(label, 5)), expected);
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

#[test]
fn a_prompt_mentions_a_timer_when_timer_or_timers_is_a_whole_word() {
    let mentions = [
        "Jarvis, set a timer for 2 minutes",
        "Chubbies, set a timer for 2 minutes.",
        "put a 10-minute TIMER on the screen",
        "what timers do I have?",
        "cancel the timer",
        "Timer: 5 minutes",
        "the egg-timers",
    ];
    let does_not = [
        "What time is it?",
        "set an alarm",
        "timeranger is a word",
        "",
    ];
    for text in mentions {
        assert!(mentions_timer(text), "{text:?}");
    }
    for text in does_not {
        assert!(!mentions_timer(text), "{text:?}");
    }
}
