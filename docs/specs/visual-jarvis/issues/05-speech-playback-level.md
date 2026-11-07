# 05: Speech playback level

Status: ready-for-agent
Stories: US2
Blocked by: None
Assignee: unassigned
Owns: src/features/speech/playback.rs, src/features/speech/playback_level.rs, tests/unit/features/speech/playback_tests.rs, tests/unit/features/speech/playback_level_tests.rs

**What to build:** Report the loudness of the audio that is actually being played, 0 to 1, about 30 times a second, so the orb moves with the voice (D9). Read it from the samples as they reach the rodio sink (a rodio source wrapper or a tap before the sink), not from synthesis output, so the level follows playback and stops when playback stops or `/stop` is used. Check the locked rodio 0.22.2 source in `~/.cargo/registry/src/` for what it already offers before writing a wrapper. Share the RMS scale with ticket 04 only if that does not make the two tickets edit each other's files: if the shared function is needed, call it from `features/listening/level.rs` once ticket 04 is done, or keep a small separate function here. The result is exposed as a bounded channel or watch value for the orb interface (ticket 07) to read; no orb types here.

## Acceptance criteria
- [ ] The level follows the played samples: a silent clip gives 0, a full-scale clip gives 1, tested with generated samples and a fake or null sink where possible.
- [ ] Updates arrive about every 33 ms (a named constant), tested with `start_paused`; nothing is emitted after playback ends or is stopped.
- [ ] A slow or missing reader never blocks playback.
- [ ] Existing playback and `/stop` tests pass unchanged.
- [ ] `cargo fmt --check`, `cargo check --locked --offline --all-targets`, `cargo test --locked --offline`, `cargo clippy --locked --offline --all-targets -- -D warnings` and `git diff --check` pass; the speech worker Python test runs if speech worker files changed.
- [ ] The report says that real playback on the Pi speaker was not proved locally.

## Comments
