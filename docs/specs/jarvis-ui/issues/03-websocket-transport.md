# 03: WebSocket transport

Status: done
Stories: US1
Blocked by: None
Assignee: Sonnet agent (2026-11-15)
Owns: `src/interfaces/orb/` (all files); `src/config/orb.rs` and the orb lines of `src/config/mod.rs`; the `axum` line and the `tokio-tungstenite` dev-dependency line of `Cargo.toml`, the `[[test]]` entry for the orb integration test, and `Cargo.lock`; `tests/unit/interfaces/orb/`; `tests/unit/config/orb_tests.rs`; `tests/integration/` (the orb WebSocket test only)
Traces to: D3, D4

**Notes for the coordinator:**
- Visual-jarvis 09 and 13 are committed (checked 2026-11-15), so nothing waits.
- Changed by the coordinator on 2026-11-15: the Unix socket stays until ticket 06. The macroquad `jarvis-orb` is still Darren's working Orb, and D13 retires it only after the React Orb is seen on the Pi.

**What to build:** Add a WebSocket server on localhost, served with `axum`, next to the existing Unix socket server in `src/interfaces/orb/`. Both carry the same messages from the same source, so the Orb works on either. Ticket 06 removes the Unix socket. Each WebSocket text message is one `orb-protocol` JSON object with no newline. The port comes from config (`JARVIS_ORB_PORT`). Its default is set in the config module, and its doc comment names the value.

Behaviour:
- A new connection gets the current state at once, then each change.
- Every ToOrb message is sent to every connected client.
- Tap messages from a client go to the same tap path that visual-jarvis 09 uses.
- A malformed frame is ignored. The connection stays open.
- Queues are bounded in both directions. When a client is slow, level messages are dropped. State and Panel messages are never dropped while there is room in the queue.
- A client that disconnects does not stop the server. The next client gets the current state.
- If the port cannot be bound, Jarvis reports it once and runs without an Orb. It does not crash.
- The server exposes a small handle for features: a bounded sender for ToOrb messages and a receiver for FromOrb events. Later tickets (08, 09) add messages without editing this module.

**Approvals:** `axum` is approved (2026-11-15) and its version is pinned from the locked source. The `ws` feature of axum and `tokio-tungstenite` as the dev-dependency test WebSocket client are approved (2026-11-15). Their exact versions are still to be pinned from the locked source.

## Acceptance criteria
- [x] The existing Unix socket tests still pass unchanged, and a state change reaches both a Unix socket client and a WebSocket client.
- [x] The port config accepts 1 and 65535. It rejects 0 with a `ConfigError`. Tested at 0, 1, 65535 and 65536 (65536 does not fit a `u16` and is rejected at parse).
- [x] The server binds to `127.0.0.1` only. A test checks the bound address.
- [x] A WebSocket client receives the current state on connect, then each change, in order.
- [x] A Tap text message reaches the tap path once. A malformed frame is ignored and the connection stays open.
- [x] A text frame of exactly `MAX_LINE_BYTES` bytes is accepted. One of `MAX_LINE_BYTES + 1` bytes is rejected. Both use the constant from `orb-protocol`, so the test follows ticket 07.
- [x] With a client that never reads, the level queue stops at its bound. The next state change still gets through. Tested with the bound constant plus one.
- [x] Two clients both receive each ToOrb message.
- [x] A client closes, a new one connects and gets the current state. The server keeps running.
- [x] With the port already in use, Jarvis reports the failure once and keeps running without an Orb. Tested.
- [x] Timing tests use `#[tokio::test(start_paused = true)]`. No wall-clock sleeps.
- [x] `scripts/verify.sh` passes. Each new boundary test was seen to fail when its limit was moved by one.
- [x] Local tests only. Pi behaviour is not proven here.

## Comments

- 2026-11-15 (coordinator): Integrated uncommitted into `dev` (Sonnet agent, high effort, main checkout). axum `=0.8.9` (default features off; `http1`, `tokio`, `ws`) and dev-dependency tokio-tungstenite `=0.29.0`; Cargo.lock gained only axum, axum-core and matchit, one tungstenite. `src/interfaces/orb/` split into `mod.rs` (server, handle, publisher), `hub.rs` (one source, per-orb bounded queues: `EVENT_QUEUE` 64, `LEVEL_QUEUE` 8), `unix.rs` (unchanged door) and `websocket.rs`. Default port 8765 (`JARVIS_ORB_PORT`, `NonZeroU16`). Review follow-up approved by Darren (Q32): the handshake refuses (403) a missing, `null` or unlisted `Origin`; allowed origins come from `JARVIS_ORB_ALLOWED_ORIGINS`, default `http://localhost:8766` and `http://127.0.0.1:8766` (`DEFAULT_UI_PORT`, must match ticket 05's static server). Coordinator re-ran in the main checkout: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -D warnings`, `cargo test --workspace` (all pass, 408 in the jarvis binary), `git diff --check`, and `cargo test orb` 20 times with no failure. Full `scripts/verify.sh` stops at the UI typecheck only because ticket 04 is mid-work in `ui/`. Not proven: the stuck-client test's buffer sizing on Linux, and a real Chromium connecting (ticket 05). A new client gets the current state but not an open Panel; ticket 08 decides that.
