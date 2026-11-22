# 02: Timers storage at schema v7

Status: done
Stories: US1, US2, US5, US7
Blocked by: 01
Assignee: unassigned
Owns: `src/storage/timers/**` (new: `mod.rs`, `types.rs`), `src/storage/migrations/v7.sql` (new), `src/storage/schema.rs` (only `SCHEMA_VERSION` and the `version < 7` step), `src/storage/mod.rs` (re-exports only), `tests/unit/storage/timers/**` (new), `tests/unit/storage/schema_tests.rs`
Traces to: D3, D4, D9, D13

**What to build:** Schema v7 and a `storage/timers/` module with its own types and every timer SQL statement (AGENTS.md "New storage domain").
- **Types:** a `TimerId` newtype, a label (optional; its length is checked by `set_timer` in ticket 04 against `orb_protocol::MAX_TIMER_LABEL_CHARS`, since storage must not depend on the screen protocol), the end time as unix milliseconds, the creation time, and a `TimerState` enum `Pending` / `Running` / `Ended` / `Cancelled` (stored as text with a CHECK, never matched on strings outside this module).
- **Table:** `timers` in `migrations/v7.sql`, as `version < 7` step inside the existing initialization transaction. Bump `SCHEMA_VERSION` to 7. Never edit an older migration.
- **Operations** (write paths use `TransactionBehavior::Immediate`):
  - insert a `Pending` timer, refused when the limit (passed in by the caller: the service uses `orb_protocol::MAX_TIMERS`, 5) of pending or running timers already exist, counted inside the same transaction as the insert (D3, pending ones included);
  - move `Pending` to `Running` (confirm), `Running` to `Ended`, and `Pending` or `Running` or `Ended` to `Cancelled`; each is one transaction and returns a typed error when the id is not in the expected state (so confirm against expiry cannot both win);
  - delete `Pending` timers that are past a given instant (their window is over);
  - list the timers shown at a given instant, soonest end first: every `Pending` and `Running` timer, and each `Ended` one whose end is at most `ENDED_TIMER_SHOWN_FOR` (2 minutes, passed in by the caller) before the instant (D19), so old ended timers never come back after a restart; get one by id; list `Running` timers whose end is at or before an instant (for D9).
- A typed `thiserror` error enum for the module; causes via `#[source]`.

## Acceptance criteria
- [x] A test in `schema_tests.rs` builds a v6 database with realistic rows (turns, sources, memories, facts) and asserts it migrates to v7 with every row kept and an empty `timers` table; `SCHEMA_VERSION` is 7; opening a v8 database is still `Unsupported`.
- [x] Boundary table: inserting the 5th pending or running timer succeeds and the 6th is refused with the typed limit error; a cancelled or ended timer does not count.
- [x] State changes: confirm moves `Pending` to `Running` once; a second confirm, and a confirm of a missing or cancelled id, return the typed error; cancel works from `Pending`, `Running` and `Ended`.
- [x] Deleting expired pending timers deletes only `Pending` rows at or before the given instant (one millisecond either side is tested), and leaves `Running` ones.
- [x] Listing returns soonest first and omits `Cancelled`; an `Ended` timer is listed when it ended exactly 2 minutes before the instant and not 1 ms earlier; the due-at-or-before query includes a timer ending exactly at the instant.
- [x] Timers survive closing and reopening the database file (temporary SQLite file with a `Drop` fixture).
- [x] No SQL outside `src/storage/`; no `unwrap()`; `scripts/verify.sh` passes.
- [ ] Before the Pi runs v7, Darren backs up `data/jarvis.sqlite3` (noted in the Comment when the ticket is deployed).

## Comments
- 2026-11-22 (coordinator): built by a Sonnet agent in a worktree from `be452bb`; reviewed and integrated into `dev`. `storage::timers` exports `TimersRepository` (`insert_pending`, `confirm`, `mark_ended`, `cancel`, `delete_pending_created_through`, `list_shown`, `get`, `list_running_due`), `Timer`, `TimerId(i64)`, `TimerState`, and `TimerError` (`LimitReached`, `NotInExpectedState { found }`, `Storage`). v7 is the `timers` table (`AUTOINCREMENT`, so an old id never names a new timer) with a CHECK on the state and an index on (state, end). The limit and the 2-minute window are parameters, and every "now" comes from the caller. Pending timers expire by `created_at` (the window starts when the timer is proposed). `archive_tests.rs` now uses version 8 as the unsupported version. Left for 03: a `timers_repository()` accessor on `ConversationArchive`. Left for 04 (corrected 2026-11-22: code reachable only from dead code is dead too, so the markers stay until `app.rs` creates the service): removing the `#[expect(dead_code)]` / `#[expect(unused_imports)]` markers. Spot-checks (agent): each of the four boundary comparisons moved by one made its test fail. Coordinator: `scripts/verify.sh` in the main checkout, all checks passed (640 tests in the main crate); `--repeat confirm_against_cancel_has_one_winner` 20 of 20. The Pi migration waits for Darren's backup (unticked criterion).
