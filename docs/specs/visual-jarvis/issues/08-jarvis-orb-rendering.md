# 08: `jarvis-orb` rendering

Status: in-review
Stories: US1, US2, US3, US7
Blocked by: 01, 03
Assignee: Claude
Owns: crates/jarvis-orb/Cargo.toml, crates/jarvis-orb/src/main.rs, crates/jarvis-orb/src/look.rs, crates/jarvis-orb/src/orb.rs, crates/jarvis-orb/tests/**

**What to build:** The `crates/jarvis-orb` program (macroquad `=0.4.16`, Wayland backend, full screen at 800x480, cursor hidden; see spec D4 and the spike result). It draws the orb of spec D1 and D2: points and links on a sphere, glyph rings of invented symbols, a warm core, light rays; Idle, Listening, Transcribing, Thinking and Speaking each look as specified, and the offline look (D3) is a dim grey breathing orb. No text on screen. Keep the state-to-look mapping (`look.rs`, plain data and pure functions: ring speeds, colours, ray strength from a level) apart from the drawing (`orb.rs`) so the mapping is unit-tested. Until ticket 09 exists the program can be driven by a stand-in: a command-line flag or keys that cycle states and set a fake level, for the spike-style check on the Pi. State and level arrive later through `orb-protocol` types. It must keep the frame rate the spike showed and must not crash when the socket is missing.

## Acceptance criteria
- [x] macroquad is pinned exactly and the program builds for aarch64 Linux; no other crate is added without Darren's approval.
- [x] A pure `look` function maps `(OrbState or Offline, level)` to the look parameters; tested as a table, including level 0.0 and 1.0 and an out-of-range level clamped.
- [ ] The five states and the offline look are visibly different on the Pi screen (checked by Darren or a screenshot) at the spike's frame rate.
- [x] No text is drawn; no popup or panic with the socket missing.
- [ ] The orb is original: it copies neither a plexus stock image nor Raphael's design.
- [x] `cargo fmt --check`, `cargo check --locked --offline --all-targets --workspace`, `cargo test --locked --offline --workspace`, `cargo clippy --locked --offline --all-targets --workspace -- -D warnings` and `git diff --check` pass.
- [x] The report says what only the Pi screen proves.

## Comments

2026-11-11 (Claude): agent finished; in the `dev` working tree, uncommitted. Review is only half done.
- **Built.** `crates/jarvis-orb` with `look.rs` (pure: `View`, `Look`, `look(view, level)`, `level_response` with `LEVEL_GAIN` 18, `Look::lerp`, `ease_factor`), `orb.rs` (drawing: 260 + 80 lattice nodes with links, two glyph rings of 10 invented marks, a core glow, 28 rays) and `main.rs` (stand-in driver: Space/Right/left click = next view, Up/Down = fake level, `A` = demo; flags `--demo`, `--windowed`, `--seconds S`, `--fps`). One extra file approved by the coordinator: `src/lib.rs` (`pub mod look;`) so `tests/` can reach `look`.
- **Verified by Claude:** `cargo fmt --all --check`, `check`, `clippy -D warnings` and `test` with `--locked --offline --workspace`, and `git diff --check`: clean. 286 root tests and 7 `orb-protocol` tests unchanged, 18 new `jarvis-orb` tests. `jarvis` has no macroquad in its tree. `Cargo.lock`: only additions, plus three dependency references renamed to versioned names (`foldhash`, `ndk-sys`) with no version change. No `draw_text`, `unwrap()`, `unsafe` or socket code in `src/`.
- **Not done yet:** Claude read the top of `look.rs` only; `orb.rs` (743 lines), `main.rs` and the tests are not reviewed line by line. Nobody has looked at the picture: the agent ran it windowed on the Mac (180 FPS, the Mac's refresh rate) but could not capture the screen. Not built or run on the Pi, so the aarch64 build, 60 FPS with this load, full screen, the cursor, the tap, and whether the six looks are distinct, original and good-looking are all open (criteria 1, 3, 4, 5, 7).

2026-11-14 (Claude): review finished; committed by Darren as `4f348d1` on 2026-11-12.
- **Read in full:** `look.rs`, `orb.rs`, `main.rs`, `lib.rs`, `Cargo.toml`; test names in `look_tests.rs`. No defect found. Geometry is built once in `Orb::new`; a frame allocates nothing; `dt` is clamped so a stall cannot make the orb jump; every `as` has its range stated. Two things only the eye can settle: rays at full strength reach 1.4 radii (255 px) and so run off the top and bottom of the 480 px screen, and the core is drawn over the front of the lattice.
- **Mac, re-run today:** `cargo fmt --all --check`, `check`, `clippy -D warnings` and `test` with `--locked --offline --workspace`, `git diff --check`: clean. 285 + 1 root, 7 `orb-protocol`, 18 `jarvis-orb` tests.
- **Pi (8 GB), run by Claude over SSH:** the release binary in `~/jarvis/target/release/` (built there on 2026-11-12, 1.5 MB) ran `--demo --fps --seconds 31` on `wayland-0`: 60.0 FPS in every one-second window after the first, longest frame 16.91 ms, 3.7% of one core, 78 MiB, exit 0, nothing on stderr. No socket code exists yet, so "socket missing" is true by construction.
- **Only the Pi screen proves (Darren's eye, criteria 3 and 5):** that it is full screen with no cursor, that the six looks are distinct and good-looking, and that the orb is original. Run: `cd ~/jarvis && ./target/release/jarvis-orb --demo` (Escape quits; a tap steps to the next look).
