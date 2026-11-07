# 08: `jarvis-orb` rendering

Status: ready-for-agent
Stories: US1, US2, US3, US7
Blocked by: 01, 03
Assignee: unassigned
Owns: crates/jarvis-orb/Cargo.toml, crates/jarvis-orb/src/main.rs, crates/jarvis-orb/src/look.rs, crates/jarvis-orb/src/orb.rs, crates/jarvis-orb/tests/**

**What to build:** The `crates/jarvis-orb` program (macroquad `=0.4.16`, Wayland backend, full screen at 800x480, cursor hidden; see spec D4 and the spike result). It draws the orb of spec D1 and D2: points and links on a sphere, glyph rings of invented symbols, a warm core, light rays; Idle, Listening, Transcribing, Thinking and Speaking each look as specified, and the offline look (D3) is a dim grey breathing orb. No text on screen. Keep the state-to-look mapping (`look.rs`, plain data and pure functions: ring speeds, colours, ray strength from a level) apart from the drawing (`orb.rs`) so the mapping is unit-tested. Until ticket 09 exists the program can be driven by a stand-in: a command-line flag or keys that cycle states and set a fake level, for the spike-style check on the Pi. State and level arrive later through `orb-protocol` types. It must keep the frame rate the spike showed and must not crash when the socket is missing.

## Acceptance criteria
- [ ] macroquad is pinned exactly and the program builds for aarch64 Linux; no other crate is added without Darren's approval.
- [ ] A pure `look` function maps `(OrbState or Offline, level)` to the look parameters; tested as a table, including level 0.0 and 1.0 and an out-of-range level clamped.
- [ ] The five states and the offline look are visibly different on the Pi screen (checked by Darren or a screenshot) at the spike's frame rate.
- [ ] No text is drawn; no popup or panic with the socket missing.
- [ ] The orb is original: it copies neither a plexus stock image nor Raphael's design.
- [ ] `cargo fmt --check`, `cargo check --locked --offline --all-targets --workspace`, `cargo test --locked --offline --workspace`, `cargo clippy --locked --offline --all-targets --workspace -- -D warnings` and `git diff --check` pass.
- [ ] The report says what only the Pi screen proves.

## Comments
