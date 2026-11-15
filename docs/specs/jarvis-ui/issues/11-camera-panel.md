# 11: Camera panel

Status: needs-info
Stories: US4
Blocked by: 10
Assignee: unassigned
Owns: none yet. Set when the ticket is unblocked.
Traces to: D8, D9

**Waiting on:**
- ~~The camera is not attached.~~ Attached 2026-11-15: Camera Module 3 (imx708). Still open: resolution and frame rate.
- The frame transport conflicts with D3. D3 says axum serves only the WebSocket. MJPEG needs an HTTP stream or binary WebSocket frames. Proposal P4, approved 2026-11-15: this ticket decides once the camera is attached; the likely answer is one `/camera` route on the same axum server. Record the choice in an ADR or in Comments. Do not start until it is decided.

**What to build (provisional):** A camera Panel. `rpicam-vid` produces MJPEG, shown as an image in the UI (D9). The process starts when the Panel is shown and stops when it is hidden or the UI disconnects. The camera Panel never times out (D8). It is added as a new `Camera` variant of the Panel enum from ticket 07, with its timeout arm in ticket 09.

Open questions for Darren: which camera model, resolution and frame rate, and the frame transport (P4).

## Acceptance criteria (provisional, to be confirmed when unblocked)
- [ ] The camera model is chosen and attached. Darren confirms in Comments.
- [ ] The frame transport is decided and recorded (P4).
- [ ] `rpicam-vid` starts on show and stops on hide or disconnect. Tested with a fake program, as the recorder tests do. No process is left behind.
- [ ] The camera Panel stays shown with paused time over one hour.
- [ ] Frame rate and CPU on the Pi are recorded against D11. A Panel may cost more while shown.
- [ ] Jarvis does not see the image. Vision is out of scope.
- [ ] No frame is stored or sent off the LAN. No cloud request is made (privacy rule).

## Comments

- 2026-11-15 (coordinator): Darren attached the camera. `rpicam-hello --list-cameras` reports imx708 (Camera Module 3); modes 1536x864 @120, 2304x1296 @56, 4608x2592 @14.35. `rpicam-vid --codec mjpeg` to /dev/null for 30 s on the idle 8 GB Pi: 1280x720 @30 = 17.9% of one core, about 76 MB RSS; 640x480 @30 = 8.8%, about 75 MB. Measured alone, without Chromium or Jarvis running. Details in `notes/ui-spike/measurements-2026-11-15.md`.
