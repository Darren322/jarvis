# 03: The workspace and the `orb-protocol` crate

Status: ready-for-agent
Stories: US1, US2, US3, US4
Blocked by: None
Assignee: unassigned
Owns: Cargo.toml, Cargo.lock, crates/orb-protocol/**

**What to build:** Turn the repo into a Cargo workspace without changing the `jarvis` crate: add a `[workspace]` table with `members = ["crates/*"]` and keep the root package. Add `crates/orb-protocol`, a library with serde as its only dependency (serde_json only as a dev-dependency for tests). It defines the messages, one JSON object per line (D6 of the spec):

- Jarvis to orb: `State` (an `OrbState` enum: `Idle`, `Listening`, `Transcribing`, `Thinking`, `Speaking`) and `Level` (a 0 to 1 value with a `LevelSource` of `Speech` or `Mic`).
- Orb to Jarvis: `Tap`.

Add a line encoder and decoder that fail with a typed error derived with `thiserror` (`thiserror = "=2.0.21"`, the same exact pin as the root crate; approved by Darren on 2026-11-07). Use `deny_unknown_fields`, a named maximum line length, and a clamp or rejection rule for levels outside 0 to 1, documented. The mapping from `AssistantState` to `OrbState` is NOT here: it lives in ticket 07, because `orb-protocol` must not know about Jarvis. Pin versions exactly as AGENTS says.

## Acceptance criteria
- [ ] The root crate still builds and all existing tests pass unchanged; `cargo test --locked --offline --workspace` passes.
- [ ] `jarvis` does not depend on macroquad, and `orb-protocol` depends only on serde and `thiserror`.
- [ ] Tests live in `crates/orb-protocol/tests/` (workspace member crates keep their tests in `crates/<name>/tests/`). Round-trip tests for every message variant, in a table.
- [ ] Boundary tests: level 0.0 and 1.0 accepted, the next value outside the range handled as documented; a line of the maximum length accepted and one byte more rejected; an unknown field and an unknown variant rejected.
- [ ] The wire format (one example per message) is documented on the types with `///`.
- [ ] `cargo fmt --check`, `cargo check --locked --offline --all-targets`, `cargo test --locked --offline`, `cargo clippy --locked --offline --all-targets -- -D warnings` and `git diff --check` pass, also with `--workspace`.
- [ ] The aarch64 build is checked or the check that could not run is reported.

## Comments
