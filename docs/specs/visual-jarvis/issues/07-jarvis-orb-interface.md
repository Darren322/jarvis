# 07: The Jarvis orb interface

Status: ready-for-agent
Stories: US1, US2, US3, US4, US5, US6
Blocked by: 03, 04, 05, 06
Assignee: unassigned
Owns: src/interfaces/orb/**, src/interfaces/mod.rs, src/interfaces/cli/repl/**, src/app.rs, src/config/orb.rs, src/config/mod.rs, tests/unit/interfaces/orb/**, tests/unit/interfaces/cli/repl/**, tests/unit/config/orb_tests.rs

**What to build:** `src/interfaces/orb/` serves the Unix socket (Jarvis is the server; the orb reconnects). It follows the AGENTS "Second interface" rule: it shares only `features/` with the CLI and never imports `App`. It:

- maps `AssistantState` to the protocol `OrbState` and sends every state change;
- sends `Level` messages about 30 times a second with the right source (`Speech` from ticket 05 while speaking, `Mic` from ticket 04 while listening), never faster than the cadence;
- reads `Tap` and turns it into a command by the real state (spec D7): Idle starts listening; Speaking stops speech then listens; Thinking drops the in-flight answer without archiving it, then listens; Listening sends now; Transcribing is ignored. The mapping is one pure function of the real state;
- feeds taps into the REPL as a second input source beside stdin, reusing T6's listening path with the VAD stop from ticket 06 combined with a tap stop. Taps end a recording by tap or by silence. The CLI `/listen` keeps Enter.

Decide and record the socket path (in a new `JARVIS_ORB_SOCKET` setting resolved to absolute once, in `config/orb.rs`): `$XDG_RUNTIME_DIR/jarvis-orb.sock` (a per-user tmpfs, cleaned at logout, mode 0700, no stale file after a reboot) or `data/` in the repo (survives a reboot so a stale file must be removed at startup, and the repo path may be long). Prefer the runtime dir unless the reasoning in the ticket Comments says otherwise. Set permissions so only the user can connect, remove a stale socket file at startup, and treat orb input as untrusted (a tap grants nothing beyond the mapped command). The orb being absent is normal: no errors, no blocking. Use bounded channels, named timeouts, `biased;` so user input wins, and drop-safe `select!` branches. High effort: this is concurrency and cancellation.

## Acceptance criteria
- [ ] The tap mapping is one pure function over `AssistantState`, tested once as a table covering all five states.
- [ ] A dropped in-flight answer is never archived and history stays whole batches (AGENTS History rule); tested.
- [ ] Wire tests over a temporary Unix socket (cleaned by a `Drop` fixture): state changes arrive in order; levels arrive about 30 Hz with `start_paused`; a malformed line, an oversized line (last accepted size and one byte more) and an unknown message are rejected without killing the server; a second orb connection and a reconnect work.
- [ ] No orb connected, or the orb disconnecting mid-run, leaves Jarvis working and silent about it.
- [ ] The interface imports nothing from `interfaces/cli` and `cli` imports nothing from `interfaces/orb` except the single input-source seam in the REPL (document it).
- [ ] Existing CLI and `/listen` behaviour and tests are unchanged.
- [ ] `cargo fmt --check`, `cargo check --locked --offline --all-targets`, `cargo test --locked --offline`, `cargo clippy --locked --offline --all-targets -- -D warnings` and `git diff --check` pass.
- [ ] The report says what only the Pi can prove.

## Comments
