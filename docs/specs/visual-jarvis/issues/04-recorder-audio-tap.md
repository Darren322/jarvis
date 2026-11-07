# 04: A recorder audio tap

Status: ready-for-agent
Stories: US3, US5
Blocked by: None
Assignee: unassigned
Owns: src/features/listening/mod.rs, src/features/listening/level.rs, tests/unit/features/listening/listening_tests.rs, tests/unit/features/listening/level_tests.rs

**What to build:** `Recorder::record` reads `arecord` output in chunks. Let observers see each chunk while recording, so that both the mic level (this epic) and the VAD (ticket 06) use one change. Use a bounded channel or a small observer parameter, whichever keeps `record`'s signature small; the existing `stop` future, the byte cap, the wall-clock deadline and kill-on-drop must behave as before. A slow or dropped observer must never block or end the recording. Add a pure function in `level.rs` that turns PCM chunks into a 0 to 1 level (RMS, with a documented scale), emitted about 30 times a second (a named constant, for example `LEVEL_INTERVAL`). `Listener::listen` stays as it is unless it must forward the tap; if so, keep its `stop` and `transcribing` parameters.

## Acceptance criteria
- [ ] Observers receive every chunk in order; a full or closed observer does not block recording or change what `Recording` returns.
- [ ] Level function: silence gives 0.0, full-scale gives 1.0, an odd trailing byte is ignored, an empty chunk gives 0.0; tested as a table.
- [ ] The level cadence is about 30 Hz, tested with `#[tokio::test(start_paused = true)]` and a fake recorder script; no real microphone.
- [ ] All existing recorder and listener tests pass unchanged, including the byte-cap, deadline, minimum-length and kill-on-drop cases.
- [ ] `cargo fmt --check`, `cargo check --locked --offline --all-targets`, `cargo test --locked --offline`, `cargo clippy --locked --offline --all-targets -- -D warnings` and `git diff --check` pass.
- [ ] The report says what was not proved without the Pi mic.

## Comments
