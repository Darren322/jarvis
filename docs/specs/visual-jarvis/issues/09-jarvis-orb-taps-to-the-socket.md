# 09: `jarvis-orb` taps and the socket link

Status: ready-for-agent
Stories: US1, US2, US3, US4, US6, US7
Blocked by: 07, 08
Assignee: unassigned
Owns: crates/jarvis-orb/src/link.rs, crates/jarvis-orb/src/input.rs, crates/jarvis-orb/src/main.rs, crates/jarvis-orb/tests/link_tests.rs

**What to build:** Connect `jarvis-orb` to Jarvis. `link.rs` connects to the Unix socket from ticket 07, reads state and level lines into the renderer, and reconnects every few seconds (a named constant) while offline, showing the offline look; it sends no taps while offline (D3). `input.rs` turns a touch or a mouse-emulated click into one `Tap` message, ignoring the duplicate when both arrive for the same press (the spike records which events fire). The orb never decides what a tap does. Ticket 08 owns `main.rs` until it is done; this ticket makes a small wiring change to it after 08 is `done`.

## Acceptance criteria
- [ ] State and level lines drive the renderer; a malformed or oversized line is ignored and does not drop the connection.
- [ ] Reconnect: after Jarvis stops, the orb shows the offline look; after Jarvis starts again it reconnects within the retry interval. Tested with a fake server on a temporary socket and `start_paused`.
- [ ] One physical tap sends exactly one `Tap`, whether touch and click both arrive or only one does; taps while offline send nothing.
- [ ] The link and tap logic are testable without a window.
- [ ] `cargo fmt --check`, `cargo check --locked --offline --all-targets --workspace`, `cargo test --locked --offline --workspace`, `cargo clippy --locked --offline --all-targets --workspace -- -D warnings` and `git diff --check` pass.
- [ ] The report says what only the Pi touchscreen proves.

## Comments
