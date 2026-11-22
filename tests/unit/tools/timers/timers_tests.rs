//! The error mapping, and the helpers the three tools' tests share: a real timer
//! service on a temporary database, with a clock that stands still at [`NOW`].
//! Nothing here sleeps, so these tests run on the plain tokio runtime.

use jiff::Zoned;
use tempfile::TempDir;
use tokio::task::JoinHandle;

use super::TimerToolError;
use crate::features::timers::{TimerServiceError, TimersHandle, WallClock, channel};
use crate::storage::{ConversationArchive, Timer, TimerError, TimerId, TimerState};

/// A Sunday evening in Singapore: the instant every tool test asks at.
pub(super) const NOW: &str = "2026-11-22T18:00:00[Asia/Singapore]";

pub(super) const MINUTE_MS: i64 = 60_000;

pub(super) fn now() -> Zoned {
    NOW.parse().expect("the test instant is valid")
}

pub(super) fn now_ms() -> i64 {
    now().timestamp().as_millisecond()
}

/// A wall clock that never moves, so no timer ends and none expires mid-test.
struct StandingClock(i64);

impl WallClock for StandingClock {
    fn now_unix_ms(&self) -> i64 {
        self.0
    }
}

/// The timer service on its own temporary database; it stops when dropped.
pub(super) struct Service {
    pub(super) handle: TimersHandle,
    task: JoinHandle<()>,
    _directory: TempDir,
}

impl Drop for Service {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Service {
    pub(super) async fn start() -> Self {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let archive = ConversationArchive::open(directory.path().join("jarvis.sqlite3"))
            .await
            .expect("archive opens");
        let repository = archive.timers_repository();
        let (handle, inbox, _events) = channel();
        let task =
            tokio::spawn(
                async move { match inbox.run(repository, StandingClock(now_ms())).await {} },
            );
        Self {
            handle,
            task,
            _directory: directory,
        }
    }

    /// Proposes and confirms a timer ending `minutes` after [`NOW`].
    pub(super) async fn run_timer(&self, label: Option<&str>, minutes: i64) -> Timer {
        let proposed = self
            .handle
            .propose(label.map(str::to_owned), now_ms() + minutes * MINUTE_MS)
            .await
            .expect("proposed");
        self.handle.confirm(proposed.id).await.expect("confirmed")
    }

    /// Waits until the shown timers satisfy `done`: the service publishes them
    /// at the end of the pass that follows each request.
    pub(super) async fn shown_until(&self, done: impl Fn(&[Timer]) -> bool) -> Vec<Timer> {
        let mut shown = self.handle.watch_shown();
        let timers = shown
            .wait_for(|timers| done(timers))
            .await
            .expect("the service is running");
        timers.clone()
    }
}

pub(super) fn timer(
    id: i64,
    label: Option<&str>,
    ends_in_minutes: i64,
    state: TimerState,
) -> Timer {
    Timer {
        id: TimerId(id),
        label: label.map(str::to_owned),
        ends_at_unix_ms: now_ms() + ends_in_minutes * MINUTE_MS,
        created_at_unix_ms: now_ms(),
        state,
    }
}

#[test]
fn the_sixth_timer_is_told_to_the_model_as_a_limit() {
    let error = TimerToolError::from(TimerServiceError::Timer(TimerError::LimitReached {
        limit: 5,
    }));
    assert!(matches!(error, TimerToolError::TooManyTimers { limit: 5 }));
    let message = error.to_string();
    assert!(message.contains("5 timers"), "{message}");
    assert!(message.contains("nothing was created"), "{message}");
}

#[test]
fn any_other_service_failure_says_the_timers_are_unavailable() {
    let error = TimerToolError::from(TimerServiceError::Unavailable);
    assert!(matches!(
        error,
        TimerToolError::Service(TimerServiceError::Unavailable)
    ));
    assert!(error.to_string().contains("unavailable"), "{error}");
}
