# 04: A recorder audio tap

Status: done
Stories: US3, US5
Blocked by: None
Assignee: unassigned
Owns: src/features/listening/mod.rs, src/features/listening/level.rs, tests/unit/features/listening/listening_tests.rs, tests/unit/features/listening/level_tests.rs

**What to build:** `Recorder::record` reads `arecord` output in chunks. Let observers see each chunk while recording, so that both the mic level (this epic) and the VAD (ticket 06) use one change. Use a bounded channel or a small observer parameter, whichever keeps `record`'s signature small; the existing `stop` future, the byte cap, the wall-clock deadline and kill-on-drop must behave as before. A slow or dropped observer must never block or end the recording. Add a pure function in `level.rs` that turns PCM chunks into a 0 to 1 level (RMS, with a documented scale), emitted about 30 times a second (a named constant, for example `LEVEL_INTERVAL`). `Listener::listen` stays as it is unless it must forward the tap; if so, keep its `stop` and `transcribing` parameters.

## Acceptance criteria
- [x] Observers receive every chunk in order; a full or closed observer does not block recording or change what `Recording` returns.
- [x] Level function: silence gives 0.0, full-scale gives 1.0, an odd trailing byte is ignored, an empty chunk gives 0.0; tested as a table.
- [x] The level cadence is about 30 Hz, tested with `#[tokio::test(start_paused = true)]` and a fake recorder script; no real microphone.
- [x] All existing recorder and listener tests pass unchanged, including the byte-cap, deadline, minimum-length and kill-on-drop cases.
- [x] `cargo fmt --check`, `cargo check --locked --offline --all-targets`, `cargo test --locked --offline`, `cargo clippy --locked --offline --all-targets -- -D warnings` and `git diff --check` pass.
- [x] The report says what was not proved without the Pi mic.

## Comments

2026-11-11 (Claude): integrated uncommitted into `dev` after one review round; committed by Claude at Darren's request on 2026-11-11.
- **Design.** `Recorder::tap()` returns a `broadcast::Receiver<AudioChunk>` (`Arc<[u8]>`), one per observer; `record`'s signature, `listener.rs` and the CLI are unchanged. The tap never blocks: an observer 64 chunks behind (`TAP_CAPACITY_CHUNKS`) loses the oldest and sees `Lagged`. `level.rs` has `rms_level` (linear RMS, all-`i16::MIN` is 1.0) and `run_level_meter`, which publishes on a `watch` every `LEVEL_INTERVAL` (33 ms) and holds a chunk's level for `LEVEL_HOLD` (250 ms) before falling to 0.0.
- **Review round.** Tapped chunks could end mid-sample, which would misalign every later chunk. Fixed: chunks are now whole samples; the spot-check with the alignment disabled failed 3 of 3 runs (agent's report).
- **Verified by Claude in the main checkout:** fmt, check, clippy at `-D warnings` and `git diff --check` clean; 271 tests pass (270 unit + 1 integration; 257 before).
- **Criterion 3 as built:** the 30 Hz cadence is tested on a paused clock driven by a broadcast sender, because a paused clock auto-advances while a child process is awaited. A separate unpaused test runs a fake script through the tap and the meter.
- **For ticket 07:** four `#[cfg_attr(not(test), expect(dead_code | unused_imports))]` markers (`Recorder::tap`, the `run_level_meter` re-export, `rms_level`, `run_level_meter`) come off when it wires them.
- **Not proved without the Pi mic:** real `arecord` chunk size and pacing (if chunks arrive about every 128 ms the level changes about 8 times a second and may look steppy), real mic levels and gain, CPU and jitter on the Pi.
