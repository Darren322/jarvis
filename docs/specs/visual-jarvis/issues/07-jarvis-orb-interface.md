# 07: The Jarvis orb interface

Status: done
Stories: US1, US2, US3, US4, US5, US6
Blocked by: 03, 04, 05, 06
Assignee: Claude
Owns: src/interfaces/orb/**, src/interfaces/mod.rs, src/interfaces/cli/repl/**, src/interfaces/cli/input.rs, src/interfaces/cli/presentation.rs, src/app.rs, src/config/orb.rs, src/config/mod.rs, src/features/control/{mod,tap}.rs, src/features/state.rs, src/features/listening/{mod,level,listener,vad}.rs, src/features/speech/{mod,output,playback,playback_level}.rs, Cargo.toml, Cargo.lock, tests/unit/interfaces/orb/**, tests/unit/interfaces/cli/**, tests/unit/config/orb_tests.rs, tests/unit/features/control/tap_tests.rs, tests/unit/features/state_tests.rs

**What to build:** `src/interfaces/orb/` serves the Unix socket (Jarvis is the server; the orb reconnects). It follows the AGENTS "Second interface" rule: it shares only `features/` with the CLI and never imports `App`. It:

- maps `AssistantState` to the protocol `OrbState` and sends every state change;
- sends `Level` messages about 30 times a second with the right source (`Speech` from ticket 05 while speaking, `Mic` from ticket 04 while listening), never faster than the cadence;
- reads `Tap` and turns it into a command by the real state (spec D7): Idle starts listening; Speaking stops speech then listens; Thinking drops the in-flight answer without archiving it, then listens; Listening sends now; Transcribing is ignored. The mapping is one pure function of the real state;
- feeds taps into the REPL as a second input source beside stdin, reusing T6's listening path with the VAD stop from ticket 06 combined with a tap stop. Taps end a recording by tap or by silence. The CLI `/listen` keeps Enter.

Decide and record the socket path (in a new `JARVIS_ORB_SOCKET` setting resolved to absolute once, in `config/orb.rs`): `$XDG_RUNTIME_DIR/jarvis-orb.sock` (a per-user tmpfs, cleaned at logout, mode 0700, no stale file after a reboot) or `data/` in the repo (survives a reboot so a stale file must be removed at startup, and the repo path may be long). Prefer the runtime dir unless the reasoning in the ticket Comments says otherwise. Set permissions so only the user can connect, remove a stale socket file at startup, and treat orb input as untrusted (a tap grants nothing beyond the mapped command). The orb being absent is normal: no errors, no blocking. Use bounded channels, named timeouts, `biased;` so user input wins, and drop-safe `select!` branches. High effort: this is concurrency and cancellation.

## Acceptance criteria
- [x] The tap mapping is one pure function over `AssistantState`, tested once as a table covering all five states.
- [x] A dropped in-flight answer is never archived and history stays whole batches (AGENTS History rule); tested.
- [x] Wire tests over a temporary Unix socket (cleaned by a `Drop` fixture): state changes arrive in order; levels arrive about 30 Hz with `start_paused`; a malformed line, an oversized line (last accepted size and one byte more) and an unknown message are rejected without killing the server; a second orb connection and a reconnect work.
- [x] No orb connected, or the orb disconnecting mid-run, leaves Jarvis working and silent about it.
- [x] The interface imports nothing from `interfaces/cli` and `cli` imports nothing from `interfaces/orb` except the single input-source seam in the REPL (document it).
- [x] Existing CLI and `/listen` behaviour and tests are unchanged.
- [x] `cargo fmt --check`, `cargo check --locked --offline --all-targets`, `cargo test --locked --offline`, `cargo clippy --locked --offline --all-targets -- -D warnings` and `git diff --check` pass.
- [x] The report says what only the Pi can prove.

## Comments

2026-11-14 (Claude): implemented by the coordinator in the main checkout; in the `dev` working tree, uncommitted.
- **Socket.** `JARVIS_ORB_SOCKET`, else `$XDG_RUNTIME_DIR/jarvis-orb.sock`, else `data/jarvis-orb.sock` under the working directory (`src/config/orb.rs`). The runtime directory wins because it is per-user, mode 0700, in memory and emptied at logout and reboot. The socket is set to mode 0600. A stale socket file is removed at start; a live one, or a file that is not a socket, is left alone and reported. If the socket cannot be opened Jarvis warns once and runs without the orb.
- **Server.** `src/interfaces/orb/mod.rs`: `OrbServer::bind` and `serve`. It sends the state on connect and on every change, in order, and a level every 33 ms while listening (source mic) or speaking (source speech). It reads lines of at most 256 bytes; a malformed, unknown or oversized line is ignored and the connection stays up. Up to 4 orbs at once; an orb that stops reading is closed after 1 s. All connections run inside the one `serve` future (no spawned task), which `App` runs beside the REPL in a `select!`, so dropping it closes everything and removes the socket file.
- **Taps.** The mapping is `features::control::tap_action(AssistantState) -> TapAction`, one pure function tested as a table. Deviation from the ticket text: it lives in `features/control/`, not in the orb interface, and the REPL calls it. Reason: the REPL owns the real state, and the orb interface sees it late, which is the race spec D7 warns about. The orb interface only passes a `Tap` into a bounded queue of one. `cli` and `orb` import nothing from each other; they meet in `App` through `Taps` (in `cli/input.rs`), `StateFeed` (in `features/state.rs`) and two level readers.
- **What a tap does.** Idle: listen, ending on silence (ticket 06) or the next tap. Listening: send now. Transcribing: ignored. Thinking: the model run is dropped, nothing is archived or added to the history, then listen. Speaking: the voice stops, then listen; a finished answer is kept, one still being written is dropped. A tap that arrives after the model has ended but while its turn is being stored keeps the answer whole (found by a flaky test: dropping there left a turn archived but not shown). The typed `/listen` still ends on Enter only.
- **Outside the original Owns** (the list above is updated): the tap mapping and `StateFeed` in `features/`, `Listener::tap`, one long-lived speech level in `SpeechOutput` that survives a rebuilt audio player (as the 2026-11-11 handoff asked), `orb-protocol` as a path dependency of the root crate, and the `wired by ticket 07` markers removed. `LevelReceiver::changed` and `AudioPlayer::level` keep a `dead_code` expectation: only tests use them.
- **Existing tests:** unchanged except for one added argument (`Taps::none()`) at eight call sites.
- **Verified by Claude on the Mac:** `cargo fmt --all --check`, `check`, `clippy -D warnings` and `test` with `--locked --offline --workspace`, and `git diff --check`: clean. Root unit tests 299 to 337, 1 integration, 7 `orb-protocol`, 19 `jarvis-orb`. Spot-checks: the line limit off by one failed the boundary test; a tap that listens without the silence stop failed its test (it ran to the 15 s cap). The tap tests ran 25 times in a row without a failure.
- **Only the Pi can prove:** the socket under `/run/user/1000` and its permissions there, levels from the real microphone and speaker at 30 Hz, a real tap ending a real recording on silence, the wait between a tap and `[listening]`, and all of it together with `jarvis-orb` (ticket 09). Nothing was run against a real model, microphone or screen.

- 2026-11-16 (coordinator): Closed with the epic (Darren, 2026-11-16: "You may also close the ticket"). Committed as `7b0e585`. The tap handling, state feed and levels are still in use; the Unix socket server was replaced by the WebSocket (jarvis-ui 03) and removed by jarvis-ui 06.
