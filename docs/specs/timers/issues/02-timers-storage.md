# 02: Timers storage at schema v7

Status: ready-for-agent
Stories: US1, US2, US5, US7
Blocked by: 01
Assignee: unassigned
Owns: `src/storage/timers/**` (new: `mod.rs`, `types.rs`), `src/storage/migrations/v7.sql` (new), `src/storage/schema.rs` (only `SCHEMA_VERSION` and the `version < 7` step), `src/storage/mod.rs` (re-exports only), `tests/unit/storage/timers/**` (new), `tests/unit/storage/schema_tests.rs`
Traces to: D3, D4, D9, D13

**What to build:** Schema v7 and a `storage/timers/` module with its own types and every timer SQL statement (AGENTS.md "New storage domain").
- **Types:** a `TimerId` newtype, a label (optional, at most `MAX_TIMER_LABEL_CHARS` characters, so 5 timers fit in one `timers` line under `orb-protocol`'s `MAX_TO_ORB_LINE_BYTES`; ticket 05 sets the value), the end time as unix milliseconds, the creation time, and a `TimerState` enum `Pending` / `Running` / `Ended` / `Cancelled` (stored as text with a CHECK, never matched on strings outside this module).
- **Table:** `timers` in `migrations/v7.sql`, as `version < 7` step inside the existing initialization transaction. Bump `SCHEMA_VERSION` to 7. Never edit an older migration.
- **Operations** (write paths use `TransactionBehavior::Immediate`):
  - insert a `Pending` timer, refused when 5 (`MAX_TIMERS`) pending or running timers already exist, counted inside the same transaction as the insert (D3, pending ones included);
  - move `Pending` to `Running` (confirm), `Running` to `Ended`, and `Pending` or `Running` or `Ended` to `Cancelled`; each is one transaction and returns a typed error when the id is not in the expected state (so confirm against expiry cannot both win);
  - delete `Pending` timers that are past a given instant (their window is over);
  - list the timers that are not `Cancelled`, soonest end first; get one by id; list `Running` timers whose end is at or before an instant (for D9).
- A typed `thiserror` error enum for the module; causes via `#[source]`.

## Acceptance criteria
- [ ] A test in `schema_tests.rs` builds a v6 database with realistic rows (turns, sources, memories, facts) and asserts it migrates to v7 with every row kept and an empty `timers` table; `SCHEMA_VERSION` is 7; opening a v8 database is still `Unsupported`.
- [ ] Boundary table: inserting the 5th pending or running timer succeeds and the 6th is refused with the typed limit error; a cancelled or ended timer does not count.
- [ ] State changes: confirm moves `Pending` to `Running` once; a second confirm, and a confirm of a missing or cancelled id, return the typed error; cancel works from `Pending`, `Running` and `Ended`.
- [ ] Deleting expired pending timers deletes only `Pending` rows at or before the given instant (one millisecond either side is tested), and leaves `Running` ones.
- [ ] Listing returns soonest first and omits `Cancelled`; the due-at-or-before query includes a timer ending exactly at the instant.
- [ ] Timers survive closing and reopening the database file (temporary SQLite file with a `Drop` fixture).
- [ ] No SQL outside `src/storage/`; no `unwrap()`; `scripts/verify.sh` passes.
- [ ] Before the Pi runs v7, Darren backs up `data/jarvis.sqlite3` (noted in the Comment when the ticket is deployed).

## Comments
