# 06: Retire jarvis-orb

Status: done
Stories: US1
Blocked by: 05
Assignee: Sonnet agent (2026-11-16)
Owns: `crates/jarvis-orb/` (delete the directory); the `jarvis-orb` line in the workspace `members` list of `Cargo.toml`; `Cargo.lock`; the Unix socket server code, its config and its tests in `src/interfaces/orb/`, `src/config/orb.rs` and `tests/unit/`
Traces to: D13

**Waiting on:** Darren has to see the React Orb on the Pi and say so in Comments. Until then, nothing is deleted.

**What to build:** Delete `crates/jarvis-orb` and remove it from the workspace members. `crates/orb-protocol` stays. The macroquad and miniquad packages leave `Cargo.lock`.

## Acceptance criteria
- [x] Darren's sign-off on the Pi Orb is recorded in Comments, with the date.
- [x] `crates/jarvis-orb/` does not exist.
- [x] The Unix socket server is removed (kept until now by ticket 03). `grep -r UnixListener src/` finds nothing.
- [x] `grep -c macroquad Cargo.lock` prints 0, and the same for `miniquad`.
- [x] `cargo check --locked --offline --all-targets` passes after the lockfile update, and `cargo check --locked --offline --workspace` passes.
- [x] Mentions of `jarvis-orb` in other docs are listed in Comments for the coordinator. This ticket does not edit files outside its Owns.
- [x] `scripts/verify.sh` passes.

## Comments

- 2026-11-16 (coordinator): Darren's sign-off on the React Orb on the Pi: "Ticket 5 is good" (2026-11-16, after using it by tap and voice on the screen). The ticket is unblocked.
- 2026-11-16 (coordinator): Sonnet agent's work integrated uncommitted into `dev`; worktree removed. Deleted: `crates/jarvis-orb/`, `src/interfaces/orb/unix.rs` and its tests, the socket-path setting (`JARVIS_ORB_SOCKET`, the `XDG_RUNTIME_DIR` fallback, `OrbConfig::socket_path`), five `OrbError` variants (only `WebSocketBind` is left) and `Hub::subscribe` (the WebSocket's `HubHandle::subscribe` is the one path now). `Cargo.toml` needed no edit (members are `crates/*`); 28 package entries left `Cargo.lock`, among them macroquad, miniquad, image, png, fontdue and glam. No dependency of the root crate became unused.
  - **Tests.** Inventory 503 to 457 (50 gone, 4 new names): 34 belonged to the deleted crate; 6 tested the socket file and its setting; `a_line_split_across_writes_is_read_as_one` only made sense for a byte stream; 8 Unix-door tests each have a WebSocket twin in `websocket_tests.rs` (checked by name); 4 two-door tests were ported to two WebSocket clients and renamed. The 30-a-second level test now reads the hub's queue under `publish_everything` instead of a socket, because arrival times over loopback are not send times under a paused clock.
  - **Review fixes by the coordinator.** A doc line rewrapped in `websocket.rs`, a needless `inbox.clone()` removed, a `_keep` binding replaced by `_mic`.
  - **Mentions outside `Owns:`, handled by the coordinator.** `tests/integration/orb/websocket_tests.rs` no longer sets `JARVIS_ORB_SOCKET`; `scripts/pi/bench_first_text.py` sets `JARVIS_ORB_PORT=8767` instead, so a benchmark Jarvis neither takes the real port nor draws on the screen; `crates/orb-protocol/src/lib.rs` and `docs/agents/pi.md` describe the WebSocket; `scripts/pi/orb_smoke.py` is deleted (it only spoke the Unix socket; the WebSocket integration test and the live kiosk cover what it checked). Left as history: the mentions in ADR 0001, the visual-jarvis and push-to-talk specs and tickets, and this spec.
  - **Verified in the main checkout.** `scripts/verify.sh --repeat interfaces::orb`: all checks passed, 20 of 20; run again after the cleanup edits: all checks passed (441 root unit tests, 102 UI tests). `grep -r UnixListener src/` finds nothing; `grep -c macroquad Cargo.lock` and `miniquad` print 0.
  - **Not verified.** Anything on the Pi. Left there for Darren: the old `~/jarvis/target/release/jarvis-orb` binary (harmless), and the Jarvis now running still creates `/run/user/1000/jarvis-orb.sock` until it is rebuilt from this change.
- 2026-11-16 (coordinator), later: deployed to the 8 GB Pi for Darren's check, with his OK ("sure rebuild jarvis then ill check then u may commit"). The uncommitted Rust change was built beside the checkout (`/tmp/jarvis-exp`, a detached worktree at `b2d1a54` plus the patch, with `CARGO_TARGET_DIR=~/jarvis/target`; 21.6 s) and Jarvis restarted in tmux session `jarvis` as `./target/debug/jarvis` from `~/jarvis`. `scripts/deploy-ui.sh` copied the new `ui/dist`, and Chromium was restarted with the kiosk launcher to load it. Checked: the server log shows the new bundle fetched (`index-DExZf_8s.js`), one WebSocket client is connected, `/run/user/1000/jarvis-orb.sock` no longer exists, and a `grim` screenshot shows the idle Orb in colour, full screen. The scratch worktree and files in `/tmp` are removed. Until the commit is pushed and pulled, `./run.sh` on the Pi would rebuild the old code from its checkout.
- 2026-11-16 (coordinator): Darren used the Pi build made from this change (tap, speech, the Dashboard) and said to commit. Done.
