# 03: The timer service

Status: done
Stories: US2, US3, US5, US6, US7
Blocked by: 02
Assignee: unassigned
Owns: `src/features/timers/**` (new: `mod.rs` and helpers), `src/features/mod.rs` (the `pub mod` line only), `src/storage/archive.rs` (a `timers_repository()` accessor only), `tests/unit/features/timers/**` (new)
Traces to: D3, D4, D6, D7, D9, D13, D14

**Coordinator implements (concurrency).** Do not give this to an agent.

**What to build:** `features/timers/`: a `TimerService` task and a cheap-to-clone `TimersHandle`, built like `PanelInbox` and `PanelHandle` (`src/features/panel/mod.rs`): a bounded request queue, one task that owns the state, no lock held across `.await`. It uses `storage/timers/` for everything durable and never touches SQL.
- **Requests** (through the handle): propose a timer (creates `Pending`, starts its `TIMER_CONFIRM_WINDOW` of 30 s), confirm, reject, cancel (by id, and by label words per D10), list with the time left, and `has_pending` (for the "yes"/"no" parse, ticket 06).
- **Events out:** a stream of `TimerEvent`s: the list changed (for the screen, ticket 07) and a timer ended (for the sound and the model run, ticket 08). Bounded channels; a slow consumer does not stall the service.
- **Firing:** sleeps until the next running timer's end (`sleep_until`), marks it `Ended`, emits the event, and keeps an ended row in the list for `ENDED_TIMER_SHOWN_FOR` (2 minutes), then drops it from the list. It re-plans when a timer is confirmed or cancelled.
- **Pending window:** an unconfirmed timer is deleted when the 30 s are over, and the list changes. Confirm against expiry: exactly one wins, decided by the storage state change.
- **Start-up (D9):** `Running` timers overdue by less than `MISSED_TIMER_GRACE` (1 hour) fire as late, carrying how late; ones overdue by 1 hour or more are marked `Ended` without a run and reported as dropped (a printed notice, through the event stream). `Pending` timers left in storage from before the restart are deleted, with no spoken line (D22).
- **Ended rows (D17, D19):** the service removes an ended row after 2 minutes and sends the new list; a cancel of an `Ended` id removes it at once and produces no spoken line.
- **Busy handling (D7):** the service only reports that a timer ended; waiting for idle belongs to the REPL (ticket 08). The list shows it as ended at once.
- **Two clocks:** end times are wall-clock unix milliseconds (they must survive a restart), while waiting uses `tokio::time`. `start_paused` moves only tokio's clock, not `Zoned::now()` or `SystemTime`, so the service takes its wall-clock source as an injected value (a closure or a small trait with a production and a test implementation) that tests advance together with tokio's clock. Stored milliseconds can reuse `storage::time::unix_time_ms`.
- **Pure planning, thin loop:** the decisions (the next wake, which pending timers' windows are over, which overdue timers fire late or are dropped, D9) are a pure function of the shown timers and one `now`, table-tested with plain integers. `tokio-rusqlite` runs each call on its own thread, so a paused tokio clock may jump during a database call; the async loop reads the wall clock once per pass, before any database await, and its paused-clock tests stay coarse.
- **Each sleep is capped** (`WALL_CLOCK_RECHECK`) and the plan recomputed from the wall clock, which absorbs an NTP correction after boot or a suspend.
- A timer fired late at start (D9) ended more than 2 minutes ago, so it is never listed as ended on the strip; it is still spoken. Accepted.
- Named constants for every limit and timeout. Every wait has a name.

## Acceptance criteria
- [x] All tests use `#[tokio::test(start_paused = true)]` with the injected wall clock; nothing waits on or reads the real wall clock.
- [x] Confirm window: confirming at 30 s sets the timer; confirming just past 30 s (30 s plus 1 ms) is refused and the timer is gone; rejecting drops it at once. Table-tested at both edges.
- [x] 5 vs 6: with 5 pending or running timers, a 6th proposal is refused with the typed error; after one is cancelled, a new one is accepted.
- [x] A timer fires at its end time (not before, not after, within the paused clock), the `ended` event carries its id and label, and the list shows it as ended.
- [x] The ended row leaves the list just after 2 minutes: present at exactly 2 minutes, gone at 2 minutes plus 1 ms (inclusive, as `list_shown` in ticket 02).
- [x] Missed timers at start: one overdue by 59 minutes 59 seconds fires late and says how late; one overdue by exactly 1 hour is dropped with a notice. Pending timers from a previous run are deleted at start.
- [x] Confirm against expiry, cancel against firing, and two timers ending in the same instant each leave one consistent result and one event per timer. Each race test passes `scripts/verify.sh --repeat <pattern>` (20 runs).
- [x] Cancelling by label: no label with one timer cancels it; a label matching one timer cancels it; zero or several matches cancel nothing and return the running list (D10).
- [x] Timers survive a restart: with the service dropped and a new one started on the same database, running timers are still planned and fire.
- [x] A dropped consumer of the event stream does not stop timers from firing.
- [x] No `unwrap()`; every future raced in `select!` is safe to drop; `scripts/verify.sh` passes.

## Comments
- 2026-11-22 (coordinator): implemented by the coordinator in the main checkout. `features/timers/` has three parts:
  - `mod.rs`: `TimersHandle` (`propose`, `confirm`, `reject`, `cancel`, `cancel_matching`, `shown`, `has_pending`, `watch_shown`), `TimersInbox::run`, `TimerEvent` (`Ended { late_by }`, `Missed`, `Expired`, `Failed`), `TimerServiceError`, `CancelOutcome`, and the `WallClock` trait with `SystemClock` (jiff). Constants: `TIMER_CONFIRM_WINDOW` 30 s, `ENDED_TIMER_SHOWN_FOR` 2 min, `MISSED_TIMER_GRACE` 1 h, `WALL_CLOCK_RECHECK` 60 s, `REQUEST_QUEUE` 4, `EVENT_QUEUE` 2 × `MAX_TIMERS`.
  - `plan.rs`: the pure decisions.
  - `matching.rs`: the D10 label match. Filler words ("the", "timer", "one", …) never narrow a match; every other word must appear in the label, ignoring case.

  One task makes every change. The shown timers are a `watch`, and events use `try_send`, so a missing listener never stops a timer. A confirm checks the window itself, so the window decides even if the expiry pass has not run. The shown list is republished at the end of the pass that reports an event, so a reader of `Ended` may briefly still see the timer as running. `ConversationArchive::timers_repository()` was added. The storage `unused_imports` marker came off (the accessor uses the re-export). The `dead_code` markers stay until ticket 04 creates the service in `app.rs`.

  Tests: 6 pure plan tables (window 30 000 vs 30 001 ms; due at the end and not 1 ms before; next wake; overdue 0, 40 min, 3 599 999 ms late vs 3 600 000 ms missed), 1 matching table (10 cases), and 15 service tests on a real temporary database with a paused clock that moves the wall clock too. The service tests never sleep: `tokio-rusqlite` calls run on their own thread, so a paused clock could jump during one. Evidence: `scripts/verify.sh` all checks passed (662 tests in the main crate); `--repeat features::timers::tests` 20 of 20; spot-checks: `>` to `>=` in the window failed 2 tests, `<` to `<=` at the grace failed 1, both restored. Only the Pi shows real timers end to end (after 04, 06, 07, 08).
