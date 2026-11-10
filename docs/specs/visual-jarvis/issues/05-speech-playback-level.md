# 05: Speech playback level

Status: done
Stories: US2
Blocked by: None
Assignee: unassigned
Owns: src/features/speech/playback.rs, src/features/speech/playback_level.rs, tests/unit/features/speech/playback_tests.rs, tests/unit/features/speech/playback_level_tests.rs

**What to build:** Report the loudness of the audio that is actually being played, 0 to 1, about 30 times a second, so the orb moves with the voice (D9). Read it from the samples as they reach the rodio sink (a rodio source wrapper or a tap before the sink), not from synthesis output, so the level follows playback and stops when playback stops or `/stop` is used. Check the locked rodio 0.22.2 source in `~/.cargo/registry/src/` for what it already offers before writing a wrapper. Share the RMS scale with ticket 04 only if that does not make the two tickets edit each other's files: if the shared function is needed, call it from `features/listening/level.rs` once ticket 04 is done, or keep a small separate function here. The result is exposed as a bounded channel or watch value for the orb interface (ticket 07) to read; no orb types here.

## Acceptance criteria
- [x] The level follows the played samples: a silent clip gives 0, a full-scale clip gives 1, tested with generated samples and a fake or null sink where possible.
- [x] Updates arrive about every 33 ms (a named constant), tested with `start_paused`; nothing is emitted after playback ends or is stopped.
- [x] A slow or missing reader never blocks playback.
- [x] Existing playback and `/stop` tests pass unchanged.
- [x] `cargo fmt --check`, `cargo check --locked --offline --all-targets`, `cargo test --locked --offline`, `cargo clippy --locked --offline --all-targets -- -D warnings` and `git diff --check` pass; the speech worker Python test runs if speech worker files changed.
- [x] The report says that real playback on the Pi speaker was not proved locally.

## Comments

2026-11-11 (Claude): integrated uncommitted into `dev`; committed by Claude at Darren's request on 2026-11-11.
- **Design.** rodio 0.22.2 has no adapter that shows samples to user code (`periodic_access` hands over the source, not the sample), so `playback_level.rs` adds `LevelTap`, a small `Source` wrapper put around each `SamplesBuffer` before `Player::append`. It publishes the RMS of each window on a `watch`; a window is one 30th of a second of played samples (`LEVEL_UPDATES_PER_SECOND`), counted in samples, not wall time. `stop()`, `finish()` and dropping the player set the level to 0.0, and audio still in flight cannot publish afterwards. Readers get a `LevelReceiver` that only copies the value, so a slow or absent reader cannot hold up the audio thread.
- **Owns exception.** One line in `src/features/speech/mod.rs` (`mod playback_level;`), allowed in the brief.
- **Verified by Claude in the main checkout (with ticket 04 also applied):** fmt, check, clippy at `-D warnings` and `git diff --check` clean; 286 tests pass, run twice (285 unit + 1 integration; 15 new from this ticket).
- **Left for ticket 07.** `AudioPlayer::level()` exists, but `SpeechOutput` (`output.rs`) does not expose it yet. Each `AudioPlayer` has its own channel and the player is rebuilt after failures, so 07 should have `output.rs` own one long-lived `PlaybackLevel` and hand it to each new player. Ticket 07's Owns therefore needs `src/features/speech/{output,playback,playback_level,mod}.rs`. Two `#[cfg_attr(not(test), expect(dead_code))]` markers (`impl LevelReceiver`, `AudioPlayer::level`) come off then.
- **Not proved locally:** real playback on the Pi speaker, and how far the measured level leads the sound (the tap sits ahead of rodio's resampler and the device buffer). The wake noise is measured too (about 0.002). The level is raw RMS; speech sits well below 1, so the orb needs its own gain.
- **Later cleanup:** this RMS and `features/listening/level.rs` (ticket 04) are separate small functions over different sample types (f32 and S16_LE bytes).
