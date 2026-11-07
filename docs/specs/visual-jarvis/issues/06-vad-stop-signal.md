# 06: VAD stop signal

Status: ready-for-agent
Stories: US5
Blocked by: 02, 04
Assignee: unassigned
Owns: src/features/listening/vad.rs, tests/unit/features/listening/vad_tests.rs

**What to build:** Use the VAD crate chosen and approved in ticket 02 to build a stop future for `Listener::listen`: it reads the recorder chunks from ticket 04, and completes after the chosen silence length following detected speech. It must not complete before speech has been heard (a tap then silence still ends at the recorder's own caps), and the recording caps (15 s, 0.3 s minimum) still apply. The stop is a plain `Future<Output = ()>`, so it can be combined with a tap stop in `select!` by ticket 07. The typed `/listen` keeps Enter. The dependency line is added to the root `Cargo.toml` with an exact pin (an allowed exception to Owns, sequenced after ticket 03). A new feature module file `vad.rs` is registered from `features/listening/mod.rs` with a single `mod vad;` line (also an allowed exception, sequenced after ticket 04). Wiring into a real run is ticket 07.

## Acceptance criteria
- [ ] The crate is the one Darren approved in ticket 02's Comments, pinned exactly, and builds for aarch64.
- [ ] Silence length and the minimum speech length are named constants with tests at the boundary: the last chunk that does not end the recording and the first that does.
- [ ] Silence only never ends the recording early; speech then silence ends it; speech, a short pause, then speech does not end it. Tested with generated or recorded PCM fixtures, not a live mic.
- [ ] The stop future is safe to drop at any `.await`.
- [ ] `cargo fmt --check`, `cargo check --locked --offline --all-targets`, `cargo test --locked --offline`, `cargo clippy --locked --offline --all-targets -- -D warnings` and `git diff --check` pass.
- [ ] The report says what the real mic and room noise may still change.

## Comments
