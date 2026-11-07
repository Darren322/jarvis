# 01: Spike: macroquad on the Pi

Status: ready-for-agent
Stories: US1, US4
Blocked by: None
Assignee: unassigned
Owns: notes/ORB_SPIKE.md, notes/orb-spike/**

**What to build:** A throwaway program that answers one question: is macroquad 0.4.16 good enough for the orb on the 8 GB Pi's 5" screen? The code stays in `notes/orb-spike/` (its own scratch crate, outside the workspace and outside `src/`). It draws a few hundred plexus nodes with links on an 800x480 window and shows the frame rate. Changes on the Pi need Darren's OK first (push-to-talk D10); read-only checks and a build in a scratch directory are fine.

## Acceptance criteria
- [ ] macroquad is pinned `=0.4.16` and builds for aarch64 Linux (on the Pi or cross-built), with the build time noted.
- [ ] `linux_backend` is set to Wayland; the window opens full screen on the 800x480 DSI-2 output under labwc with the cursor hidden.
- [ ] The measured frame rate is recorded for 100, 300 and 600 nodes (with links), plus CPU use of the process.
- [ ] A finger tap on the touchscreen arrives, both as a touch event and as a mouse-emulated click. Record which events fire.
- [ ] Idle CPU and memory are noted, so the cost next to Jarvis and the STT client is known.
- [ ] A clear recommendation: go with macroquad, or fall back to Bevy (open Pi 5 crash issue #18867), with the evidence in `notes/ORB_SPIKE.md`.
- [ ] No file outside the Owns list changed; nothing was added to the workspace or `src/`.

## Comments
