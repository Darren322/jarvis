use std::{path::PathBuf, time::Duration};

use super::{Timer, TimerError, TimerId, TimerState, TimersRepository, millis};
use crate::storage::{schema, sqlite_vec};

use TimerState::{Cancelled, Ended, Pending, Running};

const LIMIT: usize = 5;
const ENDED_SHOWN_FOR: Duration = Duration::from_secs(120);
const ENDED_SHOWN_FOR_MS: i64 = 120_000;

fn database_path(label: &str) -> PathBuf {
    let name = format!(
        "jarvis-storage-timers-{label}-{}-{}.sqlite3",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock is after Unix epoch")
            .as_nanos()
    );
    std::env::temp_dir().join(name)
}

/// Opens the file the way `ConversationArchive::open` does: the same worker, the
/// same `schema::initialize`.
async fn open_repository(path: &PathBuf) -> TimersRepository {
    sqlite_vec::register();
    let connection = tokio_rusqlite::Connection::open(path)
        .await
        .expect("database opens");
    connection
        .call(|connection| schema::initialize(connection, 1))
        .await
        .expect("schema initializes");
    TimersRepository { connection }
}

struct Fixture {
    path: PathBuf,
    timers: TimersRepository,
}

impl Fixture {
    async fn open(label: &str) -> Self {
        let path = database_path(label);
        let timers = open_repository(&path).await;
        Self { path, timers }
    }

    /// A pending timer ending at `ends_at`, created at 1,000.
    async fn pending(&self, ends_at: i64) -> Timer {
        self.timers
            .insert_pending(Some("Tea"), ends_at, 1_000, LIMIT)
            .await
            .expect("the fixture timer fits")
    }

    async fn running(&self, ends_at: i64) -> Timer {
        let timer = self.pending(ends_at).await;
        self.timers.confirm(timer.id).await.expect("confirms")
    }

    async fn ended(&self, ends_at: i64) -> Timer {
        let timer = self.running(ends_at).await;
        self.timers.mark_ended(timer.id).await.expect("ends")
    }

    async fn state_of(&self, id: TimerId) -> Option<TimerState> {
        self.timers.get(id).await.expect("get").map(|t| t.state)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn ids(timers: &[Timer]) -> Vec<TimerId> {
    timers.iter().map(|timer| timer.id).collect()
}

#[tokio::test]
async fn an_inserted_timer_is_pending_and_keeps_what_it_was_given() {
    let fixture = Fixture::open("insert").await;

    let timer = fixture
        .timers
        .insert_pending(Some("Time to sleep"), 5_000, 1_000, LIMIT)
        .await
        .expect("inserts");
    let unlabelled = fixture
        .timers
        .insert_pending(None, 6_000, 1_100, LIMIT)
        .await
        .expect("inserts");

    assert_eq!(timer.state, Pending);
    assert_eq!(timer.label.as_deref(), Some("Time to sleep"));
    assert_eq!(timer.ends_at_unix_ms, 5_000);
    assert_eq!(timer.created_at_unix_ms, 1_000);
    assert_eq!(unlabelled.label, None);
    assert_ne!(timer.id, unlabelled.id);
    assert_eq!(
        fixture.timers.get(timer.id).await.expect("get"),
        Some(timer)
    );
}

#[tokio::test]
async fn the_limit_counts_pending_and_running_timers_and_refuses_one_more() {
    // (timers already active, whether the next insert is accepted)
    for (active, accepted) in [
        (0, true),
        (LIMIT - 1, true),
        (LIMIT, false),
        (LIMIT + 1, false),
    ] {
        let fixture = Fixture::open("limit").await;
        // Seed past the limit by raising it for the fixture rows only.
        for index in 0..active {
            let ends_at = 10_000 + i64::try_from(index).expect("small");
            let timer = fixture
                .timers
                .insert_pending(None, ends_at, 1_000, usize::MAX)
                .await
                .expect("seeds");
            if index % 2 == 0 {
                fixture.timers.confirm(timer.id).await.expect("confirms");
            }
        }

        let result = fixture
            .timers
            .insert_pending(None, 99_000, 1_000, LIMIT)
            .await;

        match (accepted, result) {
            (true, Ok(timer)) => assert_eq!(timer.state, Pending, "{active} active"),
            (false, Err(TimerError::LimitReached { limit })) => assert_eq!(limit, LIMIT),
            (accepted, other) => panic!("{active} active, accepted {accepted}: {other:?}"),
        }
        let rows = fixture
            .timers
            .list_shown(0, ENDED_SHOWN_FOR)
            .await
            .expect("lists")
            .len();
        assert_eq!(rows, active + usize::from(accepted), "{active} active");
    }
}

#[tokio::test]
async fn ended_and_cancelled_timers_do_not_count_toward_the_limit() {
    let fixture = Fixture::open("limit-free").await;
    for index in 0..LIMIT {
        let ends_at = 10_000 + i64::try_from(index).expect("small");
        if index % 2 == 0 {
            fixture.ended(ends_at).await;
        } else {
            let timer = fixture.pending(ends_at).await;
            fixture.timers.cancel(timer.id).await.expect("cancels");
        }
    }

    fixture
        .timers
        .insert_pending(None, 50_000, 1_000, LIMIT)
        .await
        .expect("a full set of finished timers leaves room");
}

#[tokio::test]
async fn confirm_moves_pending_to_running_once() {
    let fixture = Fixture::open("confirm").await;
    let timer = fixture.pending(5_000).await;

    let confirmed = fixture.timers.confirm(timer.id).await.expect("confirms");
    assert_eq!(confirmed.state, Running);
    assert_eq!(confirmed.ends_at_unix_ms, 5_000);
    assert_eq!(fixture.state_of(timer.id).await, Some(Running));

    let again = fixture.timers.confirm(timer.id).await;
    assert!(matches!(
        again,
        Err(TimerError::NotInExpectedState { id, found: Some(Running) }) if id == timer.id
    ));
}

#[tokio::test]
async fn confirm_refuses_a_missing_or_cancelled_or_ended_timer() {
    let fixture = Fixture::open("confirm-refused").await;
    let cancelled = fixture.pending(5_000).await;
    fixture.timers.cancel(cancelled.id).await.expect("cancels");
    let ended = fixture.ended(6_000).await;

    for (id, found) in [
        (TimerId(9_999), None),
        (cancelled.id, Some(Cancelled)),
        (ended.id, Some(Ended)),
    ] {
        let result = fixture.timers.confirm(id).await;
        assert!(
            matches!(result, Err(TimerError::NotInExpectedState { found: f, .. }) if f == found),
            "{id:?}: {result:?}"
        );
    }
    assert_eq!(fixture.state_of(cancelled.id).await, Some(Cancelled));
    assert_eq!(fixture.state_of(ended.id).await, Some(Ended));
}

#[tokio::test]
async fn mark_ended_moves_only_a_running_timer() {
    let fixture = Fixture::open("mark-ended").await;
    let pending = fixture.pending(5_000).await;
    let running = fixture.running(6_000).await;

    assert!(matches!(
        fixture.timers.mark_ended(pending.id).await,
        Err(TimerError::NotInExpectedState {
            found: Some(Pending),
            ..
        })
    ));
    assert_eq!(
        fixture
            .timers
            .mark_ended(running.id)
            .await
            .expect("ends")
            .state,
        Ended
    );
    assert!(matches!(
        fixture.timers.mark_ended(running.id).await,
        Err(TimerError::NotInExpectedState {
            found: Some(Ended),
            ..
        })
    ));
}

#[tokio::test]
async fn confirm_against_cancel_has_one_winner() {
    let fixture = Fixture::open("race").await;
    let timer = fixture.pending(5_000).await;

    fixture.timers.cancel(timer.id).await.expect("cancel wins");
    let confirm = fixture.timers.confirm(timer.id).await;

    assert!(matches!(
        confirm,
        Err(TimerError::NotInExpectedState {
            found: Some(Cancelled),
            ..
        })
    ));
    assert_eq!(fixture.state_of(timer.id).await, Some(Cancelled));
}

#[tokio::test]
async fn cancel_works_from_pending_running_and_ended_and_only_once() {
    let fixture = Fixture::open("cancel").await;
    let pending = fixture.pending(5_000).await;
    let running = fixture.running(6_000).await;
    let ended = fixture.ended(7_000).await;

    for timer in [&pending, &running, &ended] {
        let cancelled = fixture.timers.cancel(timer.id).await.expect("cancels");
        assert_eq!(cancelled.state, Cancelled);
        assert_eq!(fixture.state_of(timer.id).await, Some(Cancelled));
    }

    for id in [pending.id, TimerId(9_999)] {
        assert!(matches!(
            fixture.timers.cancel(id).await,
            Err(TimerError::NotInExpectedState { .. })
        ));
    }
}

#[tokio::test]
async fn deleting_pending_timers_takes_those_created_at_or_before_the_instant() {
    let fixture = Fixture::open("delete-pending").await;
    // Created at 1,000 each; the instants below are one millisecond either side.
    let pending = fixture.pending(50_000).await;
    let running = fixture.running(50_000).await;
    let ended = fixture.ended(50_000).await;

    // The instant is one millisecond before creation: nothing is old enough.
    assert_eq!(
        fixture
            .timers
            .delete_pending_created_through(999)
            .await
            .expect("deletes"),
        0
    );
    assert_eq!(fixture.state_of(pending.id).await, Some(Pending));

    // Exactly at creation: the pending one goes; running and ended stay.
    assert_eq!(
        fixture
            .timers
            .delete_pending_created_through(1_000)
            .await
            .expect("deletes"),
        1
    );
    assert_eq!(fixture.state_of(pending.id).await, None);
    assert_eq!(fixture.state_of(running.id).await, Some(Running));
    assert_eq!(fixture.state_of(ended.id).await, Some(Ended));

    // A later pending timer (created at 1,000 too) survives an instant 1 ms before it
    // and goes at 1,000 + 1, which also leaves Running ones alone.
    let later = fixture.pending(60_000).await;
    assert_eq!(
        fixture
            .timers
            .delete_pending_created_through(999)
            .await
            .expect("deletes"),
        0
    );
    assert_eq!(
        fixture
            .timers
            .delete_pending_created_through(1_001)
            .await
            .expect("deletes"),
        1
    );
    assert_eq!(fixture.state_of(later.id).await, None);
    assert_eq!(fixture.state_of(running.id).await, Some(Running));
}

#[tokio::test]
async fn listing_is_soonest_first_and_omits_cancelled_timers() {
    let fixture = Fixture::open("list-order").await;
    let late = fixture.pending(9_000).await;
    let soon = fixture.running(3_000).await;
    let middle = fixture.pending(5_000).await;
    let tied = fixture.pending(5_000).await;
    let cancelled = fixture.pending(1_000).await;
    fixture.timers.cancel(cancelled.id).await.expect("cancels");

    let shown = fixture
        .timers
        .list_shown(2_000, ENDED_SHOWN_FOR)
        .await
        .expect("lists");

    assert_eq!(ids(&shown), [soon.id, middle.id, tied.id, late.id]);
    assert_eq!(shown[0].state, Running);
}

#[tokio::test]
async fn an_ended_timer_is_listed_for_two_minutes_after_its_end() {
    let now = 1_000_000;
    // (how long before `now` the timer ended, whether it is listed)
    for (ago_ms, listed) in [
        (0, true),
        (ENDED_SHOWN_FOR_MS - 1, true),
        (ENDED_SHOWN_FOR_MS, true),
        (ENDED_SHOWN_FOR_MS + 1, false),
    ] {
        let fixture = Fixture::open("list-ended").await;
        let ended = fixture.ended(now - ago_ms).await;

        let shown = fixture
            .timers
            .list_shown(now, ENDED_SHOWN_FOR)
            .await
            .expect("lists");

        assert_eq!(ids(&shown) == [ended.id], listed, "ended {ago_ms} ms ago");
    }
}

#[tokio::test]
async fn a_pending_or_running_timer_is_listed_however_overdue() {
    let fixture = Fixture::open("list-overdue").await;
    let pending = fixture.pending(10).await;
    let running = fixture.running(20).await;

    let shown = fixture
        .timers
        .list_shown(10_000_000, ENDED_SHOWN_FOR)
        .await
        .expect("lists");

    assert_eq!(ids(&shown), [pending.id, running.id]);
}

#[tokio::test]
async fn running_timers_due_include_one_ending_exactly_at_the_instant() {
    let fixture = Fixture::open("due").await;
    let at = fixture.running(5_000).await;
    let before = fixture.running(4_999).await;
    let after = fixture.running(5_001).await;
    let pending = fixture.pending(100).await;
    let ended = fixture.ended(200).await;

    let due = fixture.timers.list_running_due(5_000).await.expect("lists");

    assert_eq!(ids(&due), [before.id, at.id]);
    assert!(!ids(&due).contains(&after.id));
    assert!(!ids(&due).contains(&pending.id));
    assert!(!ids(&due).contains(&ended.id));
}

#[tokio::test]
async fn timers_survive_closing_and_reopening_the_database() {
    let fixture = Fixture::open("reopen").await;
    let running = fixture.running(5_000).await;
    let pending = fixture.pending(6_000).await;
    let cancelled = fixture.pending(7_000).await;
    fixture.timers.cancel(cancelled.id).await.expect("cancels");

    fixture
        .timers
        .connection
        .clone()
        .close()
        .await
        .expect("the first connection closes");
    let reopened = open_repository(&fixture.path).await;

    let shown = reopened
        .list_shown(0, ENDED_SHOWN_FOR)
        .await
        .expect("lists");
    assert_eq!(ids(&shown), [running.id, pending.id]);
    assert_eq!(shown[0], running);
    assert_eq!(
        reopened
            .get(cancelled.id)
            .await
            .expect("get")
            .map(|t| t.state),
        Some(Cancelled)
    );
    // Ids keep counting up, so a removed timer's id never names a later one.
    let next = reopened
        .insert_pending(None, 8_000, 1_000, LIMIT)
        .await
        .expect("inserts");
    assert!(next.id > cancelled.id);
}

#[test]
fn a_duration_in_milliseconds_saturates_instead_of_wrapping() {
    assert_eq!(millis(Duration::from_secs(120)), ENDED_SHOWN_FOR_MS);
    assert_eq!(millis(Duration::MAX), i64::MAX);
}
