# 12: Restyle the orb towards the Great Sage look

Status: in-review
Stories: US1, US2, US3, US7
Blocked by: 08
Assignee: Claude
Owns: crates/jarvis-orb/src/look.rs, crates/jarvis-orb/src/orb.rs, crates/jarvis-orb/src/main.rs, crates/jarvis-orb/tests/look_tests.rs

**Why:** On 2026-11-14 Darren saw ticket 08's orb on the Pi. He likes it and finds the six looks good, but leans towards the Great Sage / Raphael orb from "That Time I Got Reincarnated as a Slime" (he supplied a reference frame). This amends spec D1: the orb moves closer to that style. It stays an original drawing: invented glyphs, no traced artwork and no characters copied from the show.

**What to build:** Change the drawing, keep the behaviour. The reference frame is a flat, face-on mandala, not a tilted 3D sphere:

- a white-gold starburst core inside a thick, soft, glowing ring with a slightly uneven edge;
- many thin rays from the centre to past the screen edge, of mixed length and brightness;
- a band of tall, narrow, ornate glyphs standing around a circle with their tops pointing outwards, between two thin circle lines, in orange-gold;
- a second, larger band of bigger and dimmer teal glyphs further out, partly off screen, turning the other way;
- a dark background with soft teal, green and amber blobs of light drifting slowly.

Keep from ticket 08: the `View` and `Look` split (`look.rs` pure, `orb.rs` drawing), the six distinct looks and what each one means (spec D2 and D3), the easing between looks, the stand-in driver and its flags, no text, no socket code, and the frame rate. The lattice of points may stay as a smaller, dimmer sparkle inside the core ring so the mic pulse and speech motion still read. Add a `--snapshot <dir>` flag that saves one PNG per view and exits, so the picture can be reviewed without the Pi.

## Acceptance criteria
- [ ] The composition is face-on and matches the description above; the glyphs are invented and no text is drawn.
- [ ] The six views stay visibly different and keep their meaning: offline dim grey and breathing; idle dim and slow; listening reacts to the mic level; transcribing spins a glyph band fast; thinking counter-rotates the bands around a gold core; speaking drives the rays and motion from the level.
- [x] `look` stays a pure function with its table tests updated, including level 0.0, 1.0 and out-of-range values clamped.
- [ ] `--snapshot <dir>` writes six PNG files; the coordinator has looked at them.
- [x] No new crate. `cargo fmt`, `check`, `clippy -D warnings` and `test` pass for the workspace with `--locked --offline`, and `git diff --check` is clean.
- [ ] 60 FPS on the 8 GB Pi (checked with `--demo --fps`), and Darren has seen it on the Pi screen.

## Comments

2026-11-14 (Claude): direction changed by Darren during the work: a mix of Iron Man's JARVIS and the Great Sage orb, not the Great Sage look alone. A Sonnet agent built the first version; Darren then asked the coordinator to do the UI, so the agent was stopped and Claude took the code over. In the `dev` working tree, uncommitted.
- **The picture.** Face-on and centred. From the Great Sage look: a white-gold starburst inside a thick glowing ring, long thin rays, and a band of tall invented glyphs standing around it in orange-gold. From the JARVIS look: thin cyan and blue HUD rings broken into arc segments, a tick-mark gauge ring and notches, counter-rotating around the gold centre. Faint blue and teal light in the background. No text.
- **Claude's changes on top of the agent's version:** the core ring is a round, softly uneven ring (72 points, two slow waves) instead of a hard 14-sided polygon; the centre glow is wider and hotter; the glyphs are taller and packed closer (66 around the ring). `Look::lerp` is now exact at both ends (a finished transition equals its target; the test caught a rounding leftover).
- **New flag:** `--snapshot <dir>` saves one PNG per view and exits.
- **Verified by Claude on the Mac:** the workspace Verification list is clean (19 `jarvis-orb` tests). Snapshots looked at by Claude: idle, thinking and speaking. Not looked at in the last version: offline, listening, transcribing. 180 FPS windowed on the Mac, which only shows it is not slow.
- **Not done:** Claude has not read all 1,119 lines of the restyled `orb.rs`, only the parts it changed. Not built or run on the Pi, so the frame rate there and the look on the 5 inch screen are open. Darren has not seen it.
