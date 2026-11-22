# 03: The timer service

Status: ready-for-agent
Stories: US2, US3, US5, US6, US7
Blocked by: 02
Assignee: Claude
Owns: `src/features/timers/**` (new: `mod.rs` and helpers), `src/features/mod.rs` (the `pub mod` line only), `tests/unit/features/timers/**` (new)
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
- Named constants for every limit and timeout. Every wait has a name.

## Acceptance criteria
- [ ] All tests use `#[tokio::test(start_paused = true)]` with the injected wall clock; nothing waits on or reads the real wall clock.
- [ ] Confirm window: confirming at 30 s sets the timer; confirming just past 30 s (30 s plus 1 ms) is refused and the timer is gone; rejecting drops it at once. Table-tested at both edges.
- [ ] 5 vs 6: with 5 pending or running timers, a 6th proposal is refused with the typed error; after one is cancelled, a new one is accepted.
- [ ] A timer fires at its end time (not before, not after, within the paused clock), the `ended` event carries its id and label, and the list shows it as ended.
- [ ] The ended row leaves the list at 2 minutes: present at 2 minutes minus 1 ms, gone at 2 minutes.
- [ ] Missed timers at start: one overdue by 59 minutes 59 seconds fires late and says how late; one overdue by exactly 1 hour is dropped with a notice. Pending timers from a previous run are deleted at start.
- [ ] Confirm against expiry, cancel against firing, and two timers ending in the same instant each leave one consistent result and one event per timer. Each race test passes `scripts/verify.sh --repeat <pattern>` (20 runs).
- [ ] Cancelling by label: no label with one timer cancels it; a label matching one timer cancels it; zero or several matches cancel nothing and return the running list (D10).
- [ ] Timers survive a restart: with the service dropped and a new one started on the same database, running timers are still planned and fire.
- [ ] A dropped consumer of the event stream does not stop timers from firing.
- [ ] No `unwrap()`; every future raced in `select!` is safe to drop; `scripts/verify.sh` passes.

## Comments
