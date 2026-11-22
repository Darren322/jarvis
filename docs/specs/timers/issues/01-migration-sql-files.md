# 01: Move each migration's SQL into `storage/migrations/vN.sql`

Status: done
Stories: US7
Blocked by: None
Assignee: unassigned
Owns: `src/storage/schema.rs`, `src/storage/migrations/**` (new), `tests/unit/storage/schema_tests.rs`
Traces to: D13

**What to build:** The AGENTS.md trigger "about six schema versions" has been hit (`SCHEMA_VERSION` is 6). Move the SQL of each migration step out of `schema.rs` into `src/storage/migrations/vN.sql` (`v1.sql` for the first schema, then one file per step), loaded with `include_str!`. The Rust-only step `migrate_v3_to_v4` stays in Rust but its SQL text moves too. `SCHEMA_VERSION` stays 6. No change in behaviour, no new migration, no new crate. `schema.rs` keeps the version check, the order of the steps and the transaction.

## Acceptance criteria
- [x] Every migration constant's SQL lives in a `migrations/vN.sql` file and is loaded with `include_str!`; no SQL text is left inline in `schema.rs` except what `migrate_v3_to_v4` builds with parameters.
- [x] `cargo test --locked --offline -- --list` is identical before and after (same test names, same count); the ticket Comment records both counts.
- [x] The existing schema tests pass unchanged, and the schema a fresh database ends with (tables, triggers, indexes) is identical before and after: compare `sqlite_master` of a fresh v6 database built by the old and new code and record that they match.
- [x] A database built at v1 through v5 with realistic rows still migrates to the same v6 result (the existing migration tests cover it; none is edited).
- [x] No `unwrap()`; `scripts/verify.sh` passes.

## Comments
- 2026-11-22 (coordinator): built by a Sonnet agent in a worktree from `d6b5f08`; integrated uncommitted into `dev`. The seven SQL constants keep their names and doc comments and now read `include_str!("migrations/…")`: `v1.sql`, `v2.sql`, `v3.sql`, `v4_create_tables.sql`, `v4_drop_v3_tables.sql`, `v5.sql`, `v6.sql`. `migrate_v3_to_v4` and its parameterised statements stay in Rust. Evidence: `cargo test --locked --offline --workspace -- --list` gives 649 tests before and after with identical names (the agent's run). A fresh v6 database's `sqlite_master` (28 rows) is byte-identical before and after (the agent's throwaway test, reverted). Coordinator check: each file equals the old raw string except for the leading newline after `r#"`, which SQLite does not store. `scripts/verify.sh` was run by the coordinator in the main checkout: all checks passed, including the UI's 367 Vitest tests. Nothing here needs the Pi.
