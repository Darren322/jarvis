# 06: VAD stop signal

Status: done
Stories: US5
Blocked by: 02, 04
Assignee: Claude (Sonnet agent)
Owns: src/features/listening/vad.rs, tests/unit/features/listening/vad_tests.rs

**What to build:** Use the VAD crate chosen and approved in ticket 02 to build a stop future for `Listener::listen`: it reads the recorder chunks from ticket 04, and completes after the chosen silence length following detected speech. It must not complete before speech has been heard (a tap then silence still ends at the recorder's own caps), and the recording caps (15 s, 0.3 s minimum) still apply. The stop is a plain `Future<Output = ()>`, so it can be combined with a tap stop in `select!` by ticket 07. The typed `/listen` keeps Enter. The dependency line is added to the root `Cargo.toml` with an exact pin (an allowed exception to Owns, sequenced after ticket 03). A new feature module file `vad.rs` is registered from `features/listening/mod.rs` with a single `mod vad;` line (also an allowed exception, sequenced after ticket 04). Wiring into a real run is ticket 07.

## Acceptance criteria
- [x] The crate is the one Darren approved in ticket 02's Comments, pinned exactly, and builds for aarch64.
- [x] Silence length and the minimum speech length are named constants with tests at the boundary: the last chunk that does not end the recording and the first that does.
- [x] Silence only never ends the recording early; speech then silence ends it; speech, a short pause, then speech does not end it. Tested with generated or recorded PCM fixtures, not a live mic.
- [x] The stop future is safe to drop at any `.await`.
- [x] `cargo fmt --check`, `cargo check --locked --offline --all-targets`, `cargo test --locked --offline`, `cargo clippy --locked --offline --all-targets -- -D warnings` and `git diff --check` pass.
- [x] The report says what the real mic and room noise may still change.

## Comments

2026-11-14 (Claude): implemented by a Sonnet agent, reviewed and re-verified by Claude; in the `dev` working tree, uncommitted.
- **Built.** `src/features/listening/vad.rs`: `speech_ended(chunks: broadcast::Receiver<AudioChunk>)`, a plain `Future<Output = ()>`, over a pure `SpeechEnd` enum (`Waiting { speech_run }`, `Speaking { silence_run }`). `earshot = "=1.2.2"` added to the root `Cargo.toml` (approved in ticket 02; no transitive crates). Constants: `SILENCE_LENGTH_MILLISECONDS` 1000, which is 63 frames of 16 ms (1008 ms, rounded up); `MIN_SPEECH_FRAMES` 6 (96 ms of consecutive speech); `SPEECH_SCORE` 0.5. A lagged tap drops the partial frame and keeps listening; a closed tap never completes it. It owns only its receiver, detector and frame buffer, with no task and no lock, so dropping it at any `.await` is safe. Not wired: ticket 07 passes it as `stop`.
- **Extra file, approved by the coordinator:** `tests/unit/features/listening/fixtures/speech_16k_mono_s16le.raw` (64,000 bytes, 2 s of macOS `say` speech, not a person), because earshot is a neural model and does not score a tone as speech.
- **Verified by Claude on the Mac:** read the whole diff. `cargo fmt --all --check`, `check`, `clippy -D warnings` and `test` with `--locked --offline --workspace`, and `git diff --check`: clean. Root unit tests 285 to 299 (14 new), 1 integration, 7 `orb-protocol`, 18 `jarvis-orb`. `Cargo.lock` gained only the `earshot` package and the `jarvis` dependency entry. Spot-check by Claude: `SILENCE_FRAMES` minus one failed two tests (the silence boundary and speech-pause-speech); the agent reports the same for plus one and for `MIN_SPEECH_FRAMES` 5 and 7.
- **aarch64:** `earshot =1.2.2` built on the 8 GB Pi on 2026-11-14 inside the ticket 02 harness (4.9 s, `--locked`). The `jarvis` crate with this change has not been built on the Pi; the first build after a pull proves it.
- **What the real mic and room may still change:** the tests use clean synthetic speech and digital zeros. Noise that earshot scores as speech for 96 ms arms the stop, and voices in the room (a TV) keep it from ever seeing silence, so the tap and the 15 s cap end those recordings (seen on Darren's `noise1` clip). A pause longer than a second ends the recording mid-sentence (his pause clips measured 1568 and 3040 ms). The real stop lands 1008 ms after the last speech frame plus up to one read (128 ms).
- **For ticket 07:** call `Recorder::tap()` for a fresh receiver at the start of each listen, so the detector never sees audio left over from an earlier recording; then remove the two `expect` markers (`speech_ended` and its re-export in `mod.rs`).

- 2026-11-16 (coordinator): Closed with the epic (Darren, 2026-11-16: "You may also close the ticket"). Committed as `7f2fb96`; in use (Darren's tap, speak and hear runs on the Pi on 2026-11-16 ended on silence with no key).
