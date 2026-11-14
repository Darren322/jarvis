# 03: The workspace and the `orb-protocol` crate

Status: in-review
Stories: US1, US2, US3, US4
Blocked by: None
Assignee: Darren
Owns: Cargo.toml, Cargo.lock, crates/orb-protocol/**

**What to build:** Turn the repo into a Cargo workspace without changing the `jarvis` crate: add a `[workspace]` table with `members = ["crates/*"]` and keep the root package. Add `crates/orb-protocol`, a library with serde as its only dependency (serde_json only as a dev-dependency for tests). It defines the messages, one JSON object per line (D6 of the spec):

- Jarvis to orb: `State` (an `OrbState` enum: `Idle`, `Listening`, `Transcribing`, `Thinking`, `Speaking`) and `Level` (a 0 to 1 value with a `LevelSource` of `Speech` or `Mic`).
- Orb to Jarvis: `Tap`.

Add a line encoder and decoder that fail with a typed error derived with `thiserror` (`thiserror = "=2.0.21"`, the same exact pin as the root crate; approved by Darren on 2026-11-07). Use `deny_unknown_fields`, a named maximum line length, and a clamp or rejection rule for levels outside 0 to 1, documented. The mapping from `AssistantState` to `OrbState` is NOT here: it lives in ticket 07, because `orb-protocol` must not know about Jarvis. Pin versions exactly as AGENTS says.

## Acceptance criteria
- [x] The root crate still builds and all existing tests pass unchanged; `cargo test --locked --offline --workspace` passes.
- [x] `jarvis` does not depend on macroquad, and `orb-protocol` depends only on serde and `thiserror`.
- [x] Tests live in `crates/orb-protocol/tests/` (workspace member crates keep their tests in `crates/<name>/tests/`). Round-trip tests for every message variant, in a table.
- [x] Boundary tests: level 0.0 and 1.0 accepted, the next value outside the range handled as documented; a line of the maximum length accepted and one byte more rejected; an unknown field and an unknown variant rejected.
- [x] The wire format (one example per message) is documented on the types with `///`.
- [x] `cargo fmt --check`, `cargo check --locked --offline --all-targets`, `cargo test --locked --offline`, `cargo clippy --locked --offline --all-targets -- -D warnings` and `git diff --check` pass, also with `--workspace`.
- [x] The aarch64 build is checked or the check that could not run is reported.

## Comments

2026-11-11 (Claude): implemented by Claude in the main checkout (the agent's worktree started on the wrong commit and its reset waited on a permission prompt; Darren then asked Claude to do it).
- **Built.** `[workspace] members = ["crates/*"]` in the root `Cargo.toml`; `crates/orb-protocol` with `ToOrb` (`State`, `Level`), `FromOrb` (`Tap {}`), `OrbState`, `LevelSource`, `Level`, `MAX_LINE_BYTES` (256) and `ProtocolError`. Wire: `{"type":"state","state":"thinking"}`, `{"type":"level","level":0.25,"source":"mic"}`, `{"type":"tap"}`, each on one line.
- **Level rule.** `Level` is valid by construction. `Level::new` rejects anything outside 0.0 to 1.0, including NaN; `Level::clamped` is for senders with a raw measurement (NaN becomes 0); decoding rejects an out-of-range level as an invalid message.
- **Deviation, needs Darren's approval:** `serde_json` (`=1.0.151`, already locked for the root crate) is a normal dependency, not a dev-dependency. The ticket asks for a line encoder and decoder in this crate, and those need a JSON library. The second criterion stays unticked until Darren agrees. The alternative is moving `encode`/`decode` into Jarvis and `jarvis-orb`, duplicated.
- **Verified by Claude:** `cargo fmt --all --check`, `check`, `clippy -D warnings` and `test`, each with `--locked --offline` and with and without `--workspace`, and `git diff --check`: clean. Root crate 286 tests (unchanged by this ticket), `orb-protocol` 7 tests. `Cargo.lock` gained only the `orb-protocol` entry. Spot-check: changing the length check to `>=` failed the maximum-length test. `jarvis` has no macroquad in its tree.
- **Not checked:** the aarch64 build. The Mac has no `aarch64-unknown-linux-gnu` target installed and nothing was installed for it. The crate is plain serde code; the first Pi build after a pull proves it.

2026-11-14 (Claude): Darren approved `serde_json` as a normal dependency of `orb-protocol`, so the second criterion is met as built (serde, `serde_json` and `thiserror`). The aarch64 build is proven: `jarvis-orb`, which depends on `orb-protocol`, was built in release mode on the 8 GB Pi on 2026-11-12 and ran there today.
