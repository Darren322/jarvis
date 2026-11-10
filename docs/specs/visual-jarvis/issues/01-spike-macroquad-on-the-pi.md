# 01: Spike: macroquad on the Pi

Status: in-review
Stories: US1, US4
Blocked by: None
Assignee: Darren
Owns: notes/ORB_SPIKE.md, notes/orb-spike/**

**What to build:** A throwaway program that answers one question: is macroquad 0.4.16 good enough for the orb on the 8 GB Pi's 5" screen? The code stays in `notes/orb-spike/` (its own scratch crate, outside the workspace and outside `src/`). It draws a few hundred plexus nodes with links on an 800x480 window and shows the frame rate. Changes on the Pi need Darren's OK first (push-to-talk D10); read-only checks and a build in a scratch directory are fine.

## Acceptance criteria
- [x] macroquad is pinned `=0.4.16` and builds for aarch64 Linux (on the Pi or cross-built), with the build time noted.
- [ ] `linux_backend` is set to Wayland; the window opens full screen on the 800x480 DSI-2 output under labwc with the cursor hidden.
- [x] The measured frame rate is recorded for 100, 300 and 600 nodes (with links), plus CPU use of the process.
- [x] A finger tap on the touchscreen arrives, both as a touch event and as a mouse-emulated click. Record which events fire.
- [x] Idle CPU and memory are noted, so the cost next to Jarvis and the STT client is known.
- [x] A clear recommendation: go with macroquad, or fall back to Bevy (open Pi 5 crash issue #18867), with the evidence in `notes/ORB_SPIKE.md`.
- [x] No file outside the Owns list changed; nothing was added to the workspace or `src/`.

## Comments

2026-11-11 (Claude): build half done; the run at the screen is Darren's step. Checked in `notes/orb-spike/Cargo.lock`: macroquad 0.4.16 with miniquad 0.4.11. `linux_backend` is `WaylandOnly` in `src/main.rs` (not yet seen on the screen, so that criterion stays open). Agent report, not re-run by Claude: the Pi build took 27.9 s wall with no missing system library, the binary is 1.4 MB, and DSI-2 is 800x480 at 60 Hz. miniquad 0.4.11 registers `wl_touch` listeners (source read only). Left on the Pi: `~/orb-spike` (76 MB). Nothing else changed there. Run steps and the empty results table are in `notes/ORB_SPIKE.md`.

2026-11-11 (Claude, after Darren's run): **go with macroquad.** 60.0 FPS locked at 100, 300 and 600 nodes; 1.8%, 4.1% and 6.0% of one core; about 78 MiB. Finger taps arrive only as left mouse presses and never as touch events, because labwc has `mouseEmulation="yes"` for the touchscreen; ticket 09 should treat a left press as the tap. Idle Pi: 100% idle, 884 MiB used. Evidence and the table are in `notes/ORB_SPIKE.md`. Still open: Darren confirms by eye that the window was full screen with no cursor (criterion 2).
