# 09: `jarvis-orb` taps and the socket link

Status: done
Stories: US1, US2, US3, US4, US6, US7
Blocked by: 07, 08
Assignee: Claude (Sonnet agent)
Owns: crates/jarvis-orb/src/link.rs, crates/jarvis-orb/src/input.rs, crates/jarvis-orb/src/main.rs, crates/jarvis-orb/src/lib.rs, crates/jarvis-orb/tests/link_tests.rs, crates/jarvis-orb/tests/input_tests.rs

**What to build:** Connect `jarvis-orb` to Jarvis. `link.rs` connects to the Unix socket from ticket 07, reads state and level lines into the renderer, and reconnects every few seconds (a named constant) while offline, showing the offline look; it sends no taps while offline (D3). `input.rs` turns a touch or a mouse-emulated click into one `Tap` message, ignoring the duplicate when both arrive for the same press (the spike records which events fire). The orb never decides what a tap does. Ticket 08 owns `main.rs` until it is done; this ticket makes a small wiring change to it after 08 is `done`.

## Acceptance criteria
- [x] State and level lines drive the renderer; a malformed or oversized line is ignored and does not drop the connection.
- [x] Reconnect: after Jarvis stops, the orb shows the offline look; after Jarvis starts again it reconnects within the retry interval. Tested with a fake server on a temporary socket and `start_paused`.
- [x] One physical tap sends exactly one `Tap`, whether touch and click both arrive or only one does; taps while offline send nothing.
- [x] The link and tap logic are testable without a window.
- [x] `cargo fmt --check`, `cargo check --locked --offline --all-targets --workspace`, `cargo test --locked --offline --workspace`, `cargo clippy --locked --offline --all-targets --workspace -- -D warnings` and `git diff --check` pass.
- [x] The report says what only the Pi touchscreen proves.

## Comments

2026-11-14 (Claude): implemented by a Sonnet agent, reviewed and re-verified by Claude; in the `dev` working tree, uncommitted.
- **Built.** `link.rs`: a `Link` whose one background thread owns the socket (std only; this crate has no async runtime). It connects, reads state and level lines, writes taps, and on any failure shows the offline look and retries every 3 s (`RECONNECT_INTERVAL`). The render loop only copies the latest view and level and queues taps without waiting, so it never blocks a frame. Taps while offline are dropped, and the queue is emptied on connect and on disconnect so no stale tap is delivered late. Lines over 256 bytes or that do not decode are ignored and the connection stays up. `input.rs`: `TapFilter` gives one tap per press: a touch and a click in the same frame, or a click within 0.5 s of a touch, count once. `main.rs`: linked mode is the default; `--standalone` or `--demo` keeps the stand-in driver; `--socket PATH` overrides the path, which otherwise follows the same rule as Jarvis's `config/orb.rs`.
- **Tests:** 11 in `tests/link_tests.rs` (fake server on a temporary socket, real threads with short injected timings) and 1 table test of 8 rows in `tests/input_tests.rs`. The ticket asked for `start_paused`; that is tokio-only and this crate has no tokio, so the reconnect test uses a short real interval and polls with a deadline.
- **Verified by Claude on the Mac:** read `link.rs`, `input.rs` and the `main.rs` diff. `cargo fmt --all --check`, `check`, `clippy -D warnings` and `test` with `--locked --offline --workspace`, and `git diff --check`: clean, three runs in a row. 337 root unit tests, 1 integration, 7 `orb-protocol`, 19 look tests, 11 link tests, 1 input test. The agent reports both boundary spot-checks failed as they should (line limit, click window).
- **Only the Pi touchscreen and a real Jarvis prove:** that one finger tap gives exactly one `Tap` (on the Pi a tap arrives only as a left click; the touch path is untested on a real device), that a tap starts, sends or drops the right thing in a running Jarvis, that live levels move the orb smoothly, that the orb reconnects within about 3 s of a Jarvis restart, that both programs resolve the same socket under `/run/user/1000`, and the frame rate with the link thread running. None of this has been run.
- **Known limit:** dropping the `Link` waits for its thread; if a connect were ever stuck on a full listener backlog, quitting the orb would wait for it.

- 2026-11-16 (coordinator): Closed with the epic (Darren, 2026-11-16: "You may also close the ticket"). Committed as `b32c38f`. Superseded: `crates/jarvis-orb` is deleted by jarvis-ui 06; taps now come from `ui/` over the WebSocket.
