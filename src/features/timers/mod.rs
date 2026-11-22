//! Timers (timers spec): proposing, confirming, ending and cancelling them.
//!
//! One task, [`TimersInbox::run`], owns every timer decision and every write;
//! whatever sets, confirms or cancels a timer (the tools, the "yes"/"no"
//! commands, taps on the screen) uses a [`TimersHandle`]. Because one task makes
//! every change, two requests about the same timer are always handled one after
//! the other. Storage keeps the timers (schema v7), so they survive a restart.
//!
//! The service reports, and never speaks: [`TimerEvent`]s say a timer ended,
//! was missed while Jarvis was off, or was never confirmed; the REPL decides
//! what to say and when (it waits for idle, D7). The shown timers are a
//! `watch` value, so the screen and the "yes"/"no" parse read the latest list
//! without waiting.
//!
//! End times are wall-clock unix milliseconds, from an injected [`WallClock`];
//! waiting uses tokio's clock, capped at [`WALL_CLOCK_RECHECK`] so a clock
//! correction after boot is noticed.
#![cfg_attr(
    not(test),
    expect(dead_code, reason = "timers 04 creates the service in app.rs")
)]

mod matching;
mod plan;

use std::{convert::Infallible, time::Duration};

use orb_protocol::MAX_TIMERS;
use thiserror::Error;
use tokio::{
    sync::{mpsc, oneshot, watch},
    time::{Instant, sleep_until},
};

use crate::storage::{StorageError, Timer, TimerError, TimerId, TimerState, TimersRepository};

pub(crate) use matching::Matches;

/// How long a proposed timer waits for Darren's "yes" or a tap on Set (D4).
pub(crate) const TIMER_CONFIRM_WINDOW: Duration = Duration::from_secs(30);

/// How long an ended timer stays on the screen when nobody taps it away (D6, D19).
pub(crate) const ENDED_TIMER_SHOWN_FOR: Duration = Duration::from_secs(2 * 60);

/// How overdue a timer found at start may be and still fire late (D9); one
/// overdue by this or more is dropped with a notice.
pub(crate) const MISSED_TIMER_GRACE: Duration = Duration::from_secs(60 * 60);

/// The longest the service sleeps before reading the wall clock again, so a
/// clock set after boot (NTP) or a suspend moves the plan within this long.
pub(crate) const WALL_CLOCK_RECHECK: Duration = Duration::from_secs(60);

/// Requests waiting for the service. Asking waits when it is full.
const REQUEST_QUEUE: usize = 4;

/// Events waiting for the REPL. At most [`MAX_TIMERS`] timers are active, so one
/// pass produces at most that many ended or expired events; twice that leaves
/// room for a pass that runs while the last is still unread. A full queue drops
/// the event rather than stall the service.
const EVENT_QUEUE: usize = 2 * MAX_TIMERS;

/// The wall clock end times are measured against. Production reads the system
/// clock; tests move it together with tokio's paused clock.
pub(crate) trait WallClock: Send + Sync + 'static {
    /// Now, in unix milliseconds.
    fn now_unix_ms(&self) -> i64;
}

/// The system clock, through `jiff`.
pub(crate) struct SystemClock;

impl WallClock for SystemClock {
    fn now_unix_ms(&self) -> i64 {
        jiff::Timestamp::now().as_millisecond()
    }
}

/// What the service reports for the REPL to say or show.
#[derive(Debug)]
pub(crate) enum TimerEvent {
    /// A running timer reached its end. `late_by` is set when it ended while
    /// Jarvis was off and fires at start instead (D9).
    Ended {
        timer: Timer,
        late_by: Option<Duration>,
    },
    /// A running timer ended [`MISSED_TIMER_GRACE`] or more before Jarvis started;
    /// it is marked ended without a reminder (D9).
    Missed { timer: Timer },
    /// A proposed timer was never confirmed in [`TIMER_CONFIRM_WINDOW`]; it is gone.
    Expired { timer: Timer },
    /// A pass could not read or write the timers; it is tried again within
    /// [`WALL_CLOCK_RECHECK`].
    Failed(StorageError),
}

/// Why a timer request changed nothing.
#[derive(Debug, Error)]
pub(crate) enum TimerServiceError {
    #[error("the timer service is not running")]
    Unavailable,
    /// The timer was proposed more than [`TIMER_CONFIRM_WINDOW`] ago; it is gone.
    #[error("timer {} waited too long for a yes and is gone", id.0)]
    WindowOver { id: TimerId },
    /// A "no" (or Don't) for a timer that is not waiting for one.
    #[error("timer {} is not waiting for a yes or no", id.0)]
    NotPending {
        id: TimerId,
        found: Option<TimerState>,
    },
    #[error(transparent)]
    Timer(#[from] TimerError),
    #[error(transparent)]
    Storage(#[from] StorageError),
}

/// What a spoken cancel did (D10).
#[derive(Debug)]
pub(crate) enum CancelOutcome {
    /// The one timer it named is cancelled.
    Cancelled(Timer),
    /// Nothing was cancelled: no timer matched, and these could be cancelled.
    NoMatch(Vec<Timer>),
    /// Nothing was cancelled: these all matched, so Jarvis asks which.
    Several(Vec<Timer>),
}

type Reply<T> = oneshot::Sender<Result<T, TimerServiceError>>;

#[derive(Debug)]
enum Request {
    Propose {
        label: Option<String>,
        ends_at_unix_ms: i64,
        reply: Reply<Timer>,
    },
    Confirm {
        id: TimerId,
        reply: Reply<Timer>,
    },
    Reject {
        id: TimerId,
        reply: Reply<Timer>,
    },
    Cancel {
        id: TimerId,
        reply: Reply<Timer>,
    },
    CancelMatching {
        words: Option<String>,
        reply: Reply<CancelOutcome>,
    },
}

/// How features ask about timers. Cheap to clone.
#[derive(Clone)]
pub(crate) struct TimersHandle {
    requests: mpsc::Sender<Request>,
    shown: watch::Receiver<Vec<Timer>>,
}

impl TimersHandle {
    /// Proposes a timer ending at `ends_at_unix_ms`. It is only `Pending`: it runs
    /// once confirmed within [`TIMER_CONFIRM_WINDOW`] (D4). It replaces a timer
    /// still pending (D23), so at most one waits for a yes. Refused with
    /// `TimerError::LimitReached` when [`MAX_TIMERS`] are already running.
    pub(crate) async fn propose(
        &self,
        label: Option<String>,
        ends_at_unix_ms: i64,
    ) -> Result<Timer, TimerServiceError> {
        self.ask(|reply| Request::Propose {
            label,
            ends_at_unix_ms,
            reply,
        })
        .await
    }

    /// Darren said yes, or tapped Set: the pending timer runs.
    pub(crate) async fn confirm(&self, id: TimerId) -> Result<Timer, TimerServiceError> {
        self.ask(|reply| Request::Confirm { id, reply }).await
    }

    /// Darren said no, or tapped Don't: the pending timer is dropped.
    pub(crate) async fn reject(&self, id: TimerId) -> Result<Timer, TimerServiceError> {
        self.ask(|reply| Request::Reject { id, reply }).await
    }

    /// Cancels the timer `id`, from the screen (D11). An ended one is simply
    /// removed from the list (D17).
    pub(crate) async fn cancel(&self, id: TimerId) -> Result<Timer, TimerServiceError> {
        self.ask(|reply| Request::Cancel { id, reply }).await
    }

    /// Cancels the one pending or running timer `words` names (D10); with no
    /// words, the only one there is.
    pub(crate) async fn cancel_matching(
        &self,
        words: Option<String>,
    ) -> Result<CancelOutcome, TimerServiceError> {
        self.ask(|reply| Request::CancelMatching { words, reply })
            .await
    }

    /// The timers shown now, soonest end first, as far as the service has got.
    pub(crate) fn shown(&self) -> Vec<Timer> {
        self.shown.borrow().clone()
    }

    /// Whether a proposed timer is waiting for a yes or no (for `control::parse`).
    pub(crate) fn has_pending(&self) -> bool {
        self.shown
            .borrow()
            .iter()
            .any(|timer| timer.state == TimerState::Pending)
    }

    /// Follows the shown timers, for the screen (timers 07).
    pub(crate) fn watch_shown(&self) -> watch::Receiver<Vec<Timer>> {
        self.shown.clone()
    }

    async fn ask<T>(
        &self,
        request: impl FnOnce(Reply<T>) -> Request,
    ) -> Result<T, TimerServiceError> {
        let (reply, answer) = oneshot::channel();
        self.requests
            .send(request(reply))
            .await
            .map_err(|_| TimerServiceError::Unavailable)?;
        answer.await.map_err(|_| TimerServiceError::Unavailable)?
    }
}

/// What an interface holds of the timers: the handle that answers and cancels
/// them, and the events it says while idle.
pub(crate) struct TimerLink {
    pub(crate) handle: TimersHandle,
    pub(crate) events: mpsc::Receiver<TimerEvent>,
}

/// The timer service, waiting to be run.
pub(crate) struct TimersInbox {
    requests: mpsc::Receiver<Request>,
    shown: watch::Sender<Vec<Timer>>,
    events: mpsc::Sender<TimerEvent>,
}

/// A handle for features, the service that serves it, and its events.
pub(crate) fn channel() -> (TimersHandle, TimersInbox, mpsc::Receiver<TimerEvent>) {
    let (request_sender, requests) = mpsc::channel(REQUEST_QUEUE);
    let (shown, shown_receiver) = watch::channel(Vec::new());
    let (events, event_receiver) = mpsc::channel(EVENT_QUEUE);
    (
        TimersHandle {
            requests: request_sender,
            shown: shown_receiver,
        },
        TimersInbox {
            requests,
            shown,
            events,
        },
        event_receiver,
    )
}

impl TimersInbox {
    /// Serves requests and fires timers until dropped.
    ///
    /// Safe to drop at any `.await`: every change is one storage transaction,
    /// and a request is handled outside the `select!`, so only the wait itself
    /// is ever dropped.
    pub(crate) async fn run(
        mut self,
        repository: TimersRepository,
        clock: impl WallClock,
    ) -> Infallible {
        self.start(&repository, clock.now_unix_ms()).await;
        loop {
            // One reading of the wall clock decides the whole pass.
            let now = clock.now_unix_ms();
            let wake = match self.pass(&repository, now).await {
                Ok(wake) => wake,
                Err(error) => {
                    self.report(TimerEvent::Failed(error));
                    None
                }
            };
            // After a successful pass every wake is in the future; one that is not
            // means a change failed, which is tried again after the recheck.
            let wait = wake
                .filter(|&wake| wake > now)
                .map_or(WALL_CLOCK_RECHECK, |wake| wait_until(wake, now))
                .min(WALL_CLOCK_RECHECK);
            // A wait too long for the clock to add never ends early.
            let deadline = Instant::now().checked_add(wait);
            let request = tokio::select! {
                biased;
                // `None` means every handle is gone; the branch then stays off.
                Some(request) = self.requests.recv() => Some(request),
                () = sleep_until_some(deadline) => None,
            };
            if let Some(request) = request {
                self.handle(&repository, request, clock.now_unix_ms()).await;
            }
        }
    }

    /// At start: pending timers from before are gone (D22), and running ones
    /// that ended while Jarvis was off fire late or are dropped (D9).
    async fn start(&self, repository: &TimersRepository, now: i64) {
        if let Err(error) = repository.delete_pending_created_through(now).await {
            self.report(TimerEvent::Failed(error));
        }
        let due = match repository.list_running_due(now).await {
            Ok(due) => due,
            Err(error) => {
                self.report(TimerEvent::Failed(error));
                return;
            }
        };
        for timer in due {
            let overdue = plan::overdue(&timer, now);
            match repository.mark_ended(timer.id).await {
                Ok(timer) => self.report(match overdue {
                    plan::Overdue::Late(late_by) => TimerEvent::Ended {
                        timer,
                        late_by: Some(late_by),
                    },
                    plan::Overdue::Missed => TimerEvent::Missed { timer },
                }),
                Err(error) => self.report_change_failure(error),
            }
        }
    }

    /// Expires and ends what is due at `now`, publishes the shown timers, and
    /// returns when the next pass is needed.
    async fn pass(
        &self,
        repository: &TimersRepository,
        now: i64,
    ) -> Result<Option<i64>, StorageError> {
        let shown = repository.list_shown(now, ENDED_TIMER_SHOWN_FOR).await?;
        let plan::Pass { expired, due } = plan::pass(&shown, now);
        if !expired.is_empty() {
            repository
                .delete_pending_created_through(plan::expired_created_through(now))
                .await?;
            for timer in expired {
                self.report(TimerEvent::Expired { timer });
            }
        }
        for timer in due {
            match repository.mark_ended(timer.id).await {
                Ok(timer) => self.report(TimerEvent::Ended {
                    timer,
                    late_by: None,
                }),
                Err(error) => self.report_change_failure(error),
            }
        }
        let shown = repository.list_shown(now, ENDED_TIMER_SHOWN_FOR).await?;
        let wake = plan::next_wake(&shown);
        self.shown.send_if_modified(|current| {
            let changed = *current != shown;
            if changed {
                *current = shown;
            }
            changed
        });
        Ok(wake)
    }

    async fn handle(&self, repository: &TimersRepository, request: Request, now: i64) {
        match request {
            Request::Propose {
                label,
                ends_at_unix_ms,
                reply,
            } => {
                let _ =
                    reply.send(propose(repository, label.as_deref(), ends_at_unix_ms, now).await);
            }
            Request::Confirm { id, reply } => {
                let _ = reply.send(self.confirm(repository, id, now).await);
            }
            Request::Reject { id, reply } => {
                let _ = reply.send(reject(repository, id).await);
            }
            Request::Cancel { id, reply } => {
                let result = repository.cancel(id).await.map_err(TimerServiceError::from);
                let _ = reply.send(result);
            }
            Request::CancelMatching { words, reply } => {
                let _ = reply.send(cancel_matching(repository, words.as_deref(), now).await);
            }
        }
    }

    /// Confirms `id` unless its window is over at `now`, whether or not a pass has
    /// expired it yet: the window, not the order of events, decides (D4). A late
    /// confirm only refuses; the pass that follows every request expires and
    /// reports it, with any other pending timer whose window is over.
    async fn confirm(
        &self,
        repository: &TimersRepository,
        id: TimerId,
        now: i64,
    ) -> Result<Timer, TimerServiceError> {
        if let Some(timer) = repository.get(id).await?
            && timer.state == TimerState::Pending
            && plan::is_window_over(&timer, now)
        {
            return Err(TimerServiceError::WindowOver { id });
        }
        Ok(repository.confirm(id).await?)
    }

    fn report(&self, event: TimerEvent) {
        // Nobody listening, or a full queue, never stops a timer from firing.
        let _ = self.events.try_send(event);
    }

    /// A state change that found the timer already moved (a cancel won) is not
    /// a failure; anything else is reported.
    fn report_change_failure(&self, error: TimerError) {
        match error {
            TimerError::NotInExpectedState { .. } => {}
            TimerError::LimitReached { .. } => {}
            TimerError::Storage(error) => self.report(TimerEvent::Failed(error)),
        }
    }
}

/// Adds a pending timer, first dropping any timer still waiting for a yes (D23),
/// so "yes" always means the timer Jarvis asked about last. The replaced one goes
/// without a word: the new proposal's own line follows.
async fn propose(
    repository: &TimersRepository,
    label: Option<&str>,
    ends_at_unix_ms: i64,
    now: i64,
) -> Result<Timer, TimerServiceError> {
    for timer in repository.list_shown(now, ENDED_TIMER_SHOWN_FOR).await? {
        if timer.state == TimerState::Pending {
            match repository.cancel(timer.id).await {
                Ok(_) | Err(TimerError::NotInExpectedState { .. }) => {}
                Err(error) => return Err(error.into()),
            }
        }
    }
    Ok(repository
        .insert_pending(label, ends_at_unix_ms, now, MAX_TIMERS)
        .await?)
}

async fn reject(repository: &TimersRepository, id: TimerId) -> Result<Timer, TimerServiceError> {
    let found = repository.get(id).await?.map(|timer| timer.state);
    if found != Some(TimerState::Pending) {
        return Err(TimerServiceError::NotPending { id, found });
    }
    Ok(repository.cancel(id).await?)
}

async fn cancel_matching(
    repository: &TimersRepository,
    words: Option<&str>,
    now: i64,
) -> Result<CancelOutcome, TimerServiceError> {
    let shown = repository.list_shown(now, ENDED_TIMER_SHOWN_FOR).await?;
    Ok(match matching::matching(&shown, words) {
        Matches::One(timer) => CancelOutcome::Cancelled(repository.cancel(timer.id).await?),
        Matches::None(cancellable) => CancelOutcome::NoMatch(cancellable),
        Matches::Several(matched) => CancelOutcome::Several(matched),
    })
}

/// How long from `now` until `wake`, both unix milliseconds; zero when it has passed.
fn wait_until(wake: i64, now: i64) -> Duration {
    Duration::from_millis(u64::try_from(wake.saturating_sub(now)).unwrap_or(0))
}

async fn sleep_until_some(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/features/timers/timers_tests.rs"]
mod tests;
