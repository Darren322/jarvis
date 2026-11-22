# 01: Move each migration's SQL into `storage/migrations/vN.sql`

Status: ready-for-agent
Stories: US7
Blocked by: None
Assignee: unassigned
Owns: `src/storage/schema.rs`, `src/storage/migrations/**` (new), `tests/unit/storage/schema_tests.rs`
Traces to: D13

**What to build:** The AGENTS.md trigger "about six schema versions" has been hit (`SCHEMA_VERSION` is 6). Move the SQL of each migration step out of `schema.rs` into `src/storage/migrations/vN.sql` (`v1.sql` for the first schema, then one file per step), loaded with `include_str!`. The Rust-only step `migrate_v3_to_v4` stays in Rust but its SQL text moves too. `SCHEMA_VERSION` stays 6. No change in behaviour, no new migration, no new crate. `schema.rs` keeps the version check, the order of the steps and the transaction.

## Acceptance criteria
- [ ] Every migration constant's SQL lives in a `migrations/vN.sql` file and is loaded with `include_str!`; no SQL text is left inline in `schema.rs` except what `migrate_v3_to_v4` builds with parameters.
- [ ] `cargo test --locked --offline -- --list` is identical before and after (same test names, same count); the ticket Comment records both counts.
- [ ] The existing schema tests pass unchanged, and the schema a fresh database ends with (tables, triggers, indexes) is identical before and after: compare `sqlite_master` of a fresh v6 database built by the old and new code and record that they match.
- [ ] A database built at v1 through v5 with realistic rows still migrates to the same v6 result (the existing migration tests cover it; none is edited).
- [ ] No `unwrap()`; `scripts/verify.sh` passes.

## Comments
