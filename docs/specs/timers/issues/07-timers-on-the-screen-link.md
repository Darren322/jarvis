# 07: Sending timers to the screen and hearing taps

Status: ready-for-agent
Stories: US4, US5
Blocked by: 05, 06
Assignee: unassigned
Owns: `src/interfaces/orb/**`, `src/features/timers/**` (only a small adapter that turns `TimerEvent`s into `ToOrb::Timers`, if it belongs there), `src/app.rs` (only the screen wiring), `tests/unit/interfaces/orb/**`, `tests/unit/features/timers/**` (only the adapter's tests)
Traces to: D11, D14

**What to build:** The Rust side of D14, using the messages from ticket 05.
- **Out:** `ToOrb::Timers` is sent on every change of the list and whenever the UI connects. Today `hub.rs` `publish` keeps only the last `OrbState` and `subscribe` replays only that, so the hub also keeps the last `timers` message and replays it on connect, so a screen opened late or reloaded is correct at once.
- **In:** `Inbox::deliver` (`interfaces/orb/mod.rs`) routes `Tap` to its own queue and everything else to `from_orb`, which the panel loop owns alone. The timer messages get a third bounded route of their own. `FromOrb::ConfirmTimer`, `RejectTimer` and `CancelTimer` go to the same handlers as the voice commands in ticket 06 (service confirm, reject, cancel). Jarvis checks every id against storage and ignores one that is missing or in the wrong state; the UI decides nothing (jarvis-ui D2).
- **Ids and times:** storage keeps `TimerId(i64)` and `i64` unix milliseconds; the wire uses `u64`. Convert with `try_from` both ways: a wire id that does not fit is ignored like an unknown id, and a stored timer that does not fit is left out of the message (never panics).
- **Screen cancel (D11):** a `CancelTimer` from the UI cancels at once (the UI has already asked "Cancel 'Time to sleep'?" with Cancel and Keep) and Jarvis says the fixed line from ticket 06; no model run.
- **A tap on an ended row (D17):** the UI sends `cancel_timer` with the ended id; Jarvis removes the row and says nothing.

## Acceptance criteria
- [ ] A change in the service's list sends exactly one `timers` message with the full list; a newly connected UI receives the current list before anything else changes.
- [ ] Each of the three UI messages calls the matching handler once; an unknown id, a cancelled id and an id for a timer already running (for a confirm) change nothing and send the current list back.
- [ ] A confirm tap just past the 30 s window sets nothing and the strip loses the row.
- [ ] Hub tests for connect, reconnect and a dropped client pass; timing and `select!` tests pass `scripts/verify.sh --repeat <pattern>` (20 runs).
- [ ] No new crate; no `unwrap()`; `scripts/verify.sh` passes.
- [ ] Pi check (Darren's eyes and touch, after ticket 09): T1's timer appears on the screen with Set and Don't; tapping Set sets it; tapping a running row asks to cancel and cancelling works.

## Comments
