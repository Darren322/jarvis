# Jarvis repository guidance

Jarvis is a local-first voice assistant written in Rust 2024. It runs on a Raspberry Pi 5 (8 GB) and talks to a llama.cpp server on a second Pi (16 GB). Read [docs/ROADMAP.md](docs/ROADMAP.md) for current priorities and the principles that never change.

## Current work

- [docs/specs/visual-jarvis/spec.md](docs/specs/visual-jarvis/spec.md) (orb, tap-to-talk, VAD) closed on 2026-11-16; its last ticket, 13 (sound cues), was closed by Darren on 2026-11-21 as it stands, without the listening-cue fix. Each ticket's `Status:` line in `issues/` is the progress record for every spec.
- Active spec: [docs/specs/fast-memory/spec.md](docs/specs/fast-memory/spec.md) (search memory only on demand, a filler line, a cached facts list). Its `Resume here:` line says what to do first.
- As of 2026-11-15: fast-memory 08 (longer conversation, cache warm-up) is done and on the Pi, and the Gemma server now runs 2 slots of 5,120 tokens (since 2026-11-21; 4,096 before); visual-jarvis 13 (state sounds: WAV cues in `data/sounds/`, not in git) is committed, on the Pi and heard by Darren ("sound effect is good"), but stays in review: the listening cue delays the recorder and cuts off the start of his speech, and the fix depends on a beep test whose result is on the 16 GB Pi; Jarvis now has a cheeky prompt and the Alan voice; resume from `notes/HANDOFF_2026-11-15b.md`.
- Second active spec, started 2026-11-15: [docs/specs/jarvis-ui/spec.md](docs/specs/jarvis-ui/spec.md) replaces the macroquad `jarvis-orb` with a React UI in Chromium kiosk so Jarvis can show panels (stats dashboard first, camera later). Terms are in `GLOSSARY.md`; the runtime choice and Pi numbers are in `docs/adr/0001-web-ui-in-chromium.md`. As of 2026-11-20: 01 to 10 and 13 to 15 are done (13 and 15 tried by voice on the Pi). Jarvis on the 8 GB Pi now runs as the systemd user service `jarvis.service` (release build; `docs/agents/pi.md`). 16 (Jarvis can say the stats it shows, `c4ca182`) was closed by Darren on 2026-11-21. 11 (camera) closed on 2026-11-21. 12 (catalog panels: a flat `build_panel` tool) closed on 2026-11-21. 17 (a timer) moved to the timers spec on 2026-11-22; nothing in jarvis-ui is open. UI tickets go to Opus agents.
- Active spec, written 2026-11-22: [docs/specs/timers/spec.md](docs/specs/timers/spec.md) (one-off voice timers that Darren confirms before they are set, a strip at the top right of the screen, a model run when a timer ends, schema v7). Decisions D1 to D22 were settled by grilling; tickets 01 to 09 are written and `ready-for-agent`. Next: 01 and 05 in parallel (see the spec's `Resume here:`). Google Calendar becomes its own spec after this one (it needs an OAuth login and a cloud opt-in).
- [docs/specs/current-time/spec.md](docs/specs/current-time/spec.md) (a `current_time` tool, `jiff = "=0.2.37"`) closed on 2026-11-21 after Darren's voice check on the Pi. A follow-up the same day (`525c69e`, `28ffb63`) makes it say 12-hour time and return only the part asked for (`part`: time, date or all); Darren confirmed it on the Pi.
- As of 2026-11-21: fast-memory 03 to 06 and 12 (the facts list: stored and sent with every request, "remember that", noted while idle, listed and forgotten, by voice too; through `5b16e6a`, schema v6 on the 8 GB Pi, backup `data/jarvis.sqlite3.bak-2026-11-21`) were closed by Darren on 2026-11-21 after his voice check. fast-memory 07 and 11 are wontfix. fast-memory 09 was seen on the Pi and closed the same day; 13 (warm the cache when Jarvis starts) and 14 (note only sentences about the user) were added and closed that evening, so every fast-memory ticket is done or wontfix. All 9 facts were deleted on the Pi at Darren's request (backup `data/jarvis.sqlite3.bak-2026-11-21b`). jarvis-ui 11 (camera) closed on 2026-11-21 (the sideways picture is left for later, Q14 in the ticket). The roadmap gained steps 7 to 12 on 2026-11-21 (camera, turn reports, Jarvis knows itself, touching the Panels, questions about its own code, Jarvis changes itself); each starts with a grilling. Latest handoff: the newest `notes/HANDOFF_*.md`.
- [docs/specs/push-to-talk.md](docs/specs/push-to-talk.md) closed on 2026-11-11; its "Resume here" lists the non-blocking leftovers. Latest handoff: the newest `notes/HANDOFF_*.md`.
- Added 2026-11-16: fast-memory 09 (a fresh conversation after 30 minutes of quiet) is committed (`cf112ae`) and on the Pi since 2026-11-21, not yet seen after a real 30-minute gap. fast-memory 10 (a tool run emptied the prompt cache) is done (`d94f962`; measured 9.6 s to 0.8 s against Gemma, and confirmed with Jarvis on the Pi on 2026-11-20). The roadmap's recognition rule is rewritten: recognition runs on the AI HAT, recognizing is not authenticating, and per-person spaces are phases 12C and 12D (design by grilling first).
- At the end of every session, update the ticket `Status:` lines, the spec's `Resume here:` line and this section (and the roadmap if a step finished) so the next session starts from current facts.

## Engineering practice

- **Don't reinvent the wheel.** Before writing a helper, check std, the pinned crates (their locked source in `~/.cargo/registry/src/`), existing modules in this repo, and standard tools (`arecord`, `systemd`, `nmcli`). Prefer them over custom code. Before each ticket, survey which existing crates, crate features, servers or frameworks would cut the most boilerplate, and propose them with trade-offs. A new crate or feature still needs Darren's approval.
- **SOLID, applied with the rules below.** One responsibility per module (the Layout table). Extend by adding variants, tools and rows rather than editing unrelated code. Keep interfaces small, and depend on a trait only at a real seam with two implementations (e.g. production and test). "Add things when a concrete need exists" still wins over speculative abstraction.
- **Pressman-style process.** Requirements live in a spec under `docs/specs/`. Each ticket traces to it and states acceptance criteria. Design before code; verify (the Verification list) and review every ticket before it counts as done; record the outcome in the spec's Status table.
- **Skills to use.**
  - Implementing: `rust-skills` (rules index in `~/.claude/skills/rust-skills/`), `tdd` / `test-driven-development`, `incremental-implementation`, and `source-driven-development` for library APIs.
  - Reviewing: `code-review-and-quality`, plus `doubt-driven-development` for risky or irreversible steps.
  - Interfaces: `api-and-interface-design` / `codebase-design`.
- **Planning flow (new specs).** `/grilling` on the epic → user stories → `/to-spec` → `/to-tickets` → `/implement-spec`. Darren reviews and commits every ticket. During grilling, write Darren's own example sentences into a table with what each one does, and check every decision against them before a ticket starts.

## Agent skills

### Issue tracker

Local markdown under `docs/specs/<feature>/`: `spec.md` (the epic and its user stories), plus one file per ticket in `issues/`. See `docs/agents/issue-tracker.md`.

### Triage labels

The five default roles (`needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`), written as each ticket's `Status:` line. See `docs/agents/triage-labels.md`.

### The Pis

Hosts, how each service is started and with which flags, and how to measure without touching Darren's data: `docs/agents/pi.md`. Read it before any SSH, benchmark or deployment step.

### Domain docs

Single-context: `GLOSSARY.md` and `docs/adr/` at the repo root, created lazily. See `docs/agents/domain.md`.

## Sources of truth

- The working tree, `Cargo.lock`, and `docs/` are the project facts. `notes/` (untracked) holds handoffs and the archived full roadmap: history, not rules.
- `rig-agent` and `rig-core` are pinned to exactly `0.42.0`. Check every library API against the locked version's source or docs (`~/.cargo/registry/src/`), not against memory, newer docs, or unversioned examples.

## Working with Darren

- For plans and material design choices, use the `grilling` skill to settle open decisions before acting. Settled decisions and explicit authorizations persist; don't ask again.
- Default learning workflow is hands-on Rust: explain one concept, offer one small patch with its rationale, then wait. Follow the requested scope; don't implement a whole phase when asked for a small change or guided learning, and don't start a phase just because it is documented.
- **Branches.** All work happens on `dev`. `main` holds only stable releases that Darren cuts; never commit to it, merge into it or reset it. Agent worktrees must start from `dev`: check `git log --oneline -1` first and `git reset --hard dev` inside the worktree if it started elsewhere.
- Preserve user changes. Don't commit, push, stage, or rewrite unrelated files unless asked. When asked to commit, make one commit per coherent step with a conventional prefix (`feat`, `fix`, `refactor`, `test`, `docs`, `build`, `chore`) and a body that says what changed and why. Commits are Darren's: use the repo's git identity as configured and add no `Co-Authored-By` or other assistant attribution line. Never push unless asked.
- When delegating to sub-agents, give each explicit, non-overlapping file ownership, then integrate and review their work. Agents in worktrees build with `CARGO_TARGET_DIR=<main checkout>/target`, so dependency builds are shared rather than duplicated (about 2 GB per worktree otherwise). Remove each worktree once its changes are integrated. Run agents at `high` effort for very complicated features (concurrency, cancellation, streaming pipelines, multi-module wiring) and at default effort for simple, contained tickets.
- **Delivering a ticket** (coordinator's loop; details in `docs/agents/issue-tracker.md`):
  1. Take every frontier ticket (all blockers `done` or integrated) whose `Owns:` lists don't overlap, and run them in parallel. Set `Status: in-progress` and `Assignee:` in the main checkout; implementers don't edit ticket files.
  2. Each brief carries the ticket text, the spec decisions it cites, the files the agent owns, and a pointer to `docs/agents/implementer-brief.md`, which holds the standing rules (stay inside `Owns:`, leave git alone, stop and report, verify, report facts).
  3. Review before integrating: read the whole diff against the acceptance criteria, and run `scripts/verify.sh` yourself in the main checkout. An agent's "all green" is a claim, not evidence. A new test of a `select!` race, cancellation or timing passes review only after `scripts/verify.sh --repeat <pattern>` (20 runs): a race that passes once proves nothing.
  4. Integrate uncommitted into `dev`, tick the verified criteria, add a dated Comment with the evidence, set `Status: in-review`, and remove the worktree. Darren commits; then the ticket is `done`. A ticket blocked only on in-review tickets may start from the integrated working tree.
  5. Who implements: Haiku agents take pure reading and mechanical edits where the agent works out the text itself (surveys, renames across files); the coordinator applies ticket and doc bookkeeping directly when it already has the exact text, since dictating it to an agent costs more than the edit; Sonnet agents take contained tickets; the coordinator implements a ticket itself, in the main checkout, when it is hard (concurrency, cancellation, cross-module wiring), when it is visual work judged by eye (the orb's look), or when an agent is blocked or has failed a review round. The same review and Verification steps apply either way.
  6. Batch what needs Darren (approvals, Pi changes, his voice or eyes) into one clear list instead of stopping at each one; keep working on everything that doesn't need him.
- The repo is a Cargo workspace (root `jarvis` crate plus `crates/*`). Run the Verification commands with `--workspace` (and `cargo fmt --all --check`) so member crates are covered. Scratch crates outside `crates/` (such as `notes/*-spike/`) carry their own empty `[workspace]` table so the root workspace ignores them.
- Near a usage limit, write a factual handoff under `notes/`: completed work vs. proposals, changed files, decisions, verification evidence, source revision, and the exact next small action.

## Layout

| Path | Owns |
| --- | --- |
| `src/main.rs` | Tokio runtime, config load, `App`, bounded shutdown. Nothing else. |
| `src/app.rs` | Assembling clients and services; startup. No parsing, REPL flow, or domain logic. |
| `src/config/` | Environment parsing, one file per settings section (`local_llm.rs`, `tts.rs`, ...). Every path is resolved to absolute once, here. |
| `src/clients/` | Provider construction and transport (`local_llm`, `remote_embeddings`) and the shared `local_endpoint` guard. |
| `src/tools/` | Rig tools and `ALLOWED_TOOLS`. |
| `src/features/assistant/` | One bounded Rig run: `Assistant`, policy, policy hook, run observer and report. |
| `src/features/control/` | Interface-neutral commands: `parse`, the `COMMANDS` help table, and `MemoryControl` (memory commands and the "forget that" focus) returning `Notice`s. |
| `src/features/conversation/` | Session history and archive coordination. |
| `src/features/memory/` | Recall, imports, forgetting, idle embedding (`MemoryService`, `Embedder`). |
| `src/features/speech/` | Synthesis worker, playback, cleanup. |
| `src/interfaces/cli/` | Terminal input, the REPL loop, streaming display, speech control, printing notices. No command logic. |
| `src/storage/` | SQLite schema, migrations, chunking, and every SQL statement. Each storage module keeps its own types (`archive.rs`, `memory_repository/types.rs`); `storage/mod.rs` re-exports them. |

- Dependencies point one way: `interfaces` → `features` → `storage` / `clients` / `tools`. Storage and features never import `App` or the CLI; the CLI never depends on `App`.
- `foo.rs` for a leaf module; `foo/mod.rs` only when it has submodules. A feature's main type may live in its `mod.rs` (`Assistant`, `MemoryService`). Don't name a file after its parent (`assistant/assistant.rs` trips `clippy::module_inception`).
- Don't add `services/`, `adapters/`, or other parents just to relocate files, empty placeholder files or directories, or layers that only forward calls. Split a file when it holds two separable concerns, not because of its line count.
- Production code lives in `src/`; tests live in `tests/` (see Testing).
- **Adding a command:** add a variant, a `parse` arm, and a `COMMANDS` row in `features/control/command.rs`, and handle it in `features/control/`. An interface changes only if the command controls the interface itself (`/stop`, `/exit`). Command handlers return `Notice`s; they never print.

### When to restructure

Split or add structure when one of these triggers is hit, and not before:

| Area | Trigger | Then |
| --- | --- | --- |
| `src/tools/` | About 8 tools, or measured wrong tool choices | Reached 2026-11-22 (8 tools with the timers). Group by domain in the source tree (`src/tools/timers/` is the first) but keep one flat `ALLOWED_TOOLS`, and offer every tool on every request: changing the tool list changes the start of the prompt and empties Gemma's prompt cache (fast-memory 10: 9.6 s vs 0.8 s). Narrow per request with `RequestPatch::active_tools` only if wrong tool choices are measured, accepting the cold cache then. |
| Tool permissions | The first tool that writes or acts (roadmap 8B) | Done 2026-11-22 (timers 04): `ToolRule` has `ToolAccess::{NoApproval, NeedsApproval}`; `set_timer` is the only `NeedsApproval` and only proposes. The approval prompt belongs to the interface, not the tool (timers 06). |
| Tool clients | A tool needs the network | Transport in `src/clients/` with the `local_endpoint.rs` protections; a thin `Tool` wrapper in `src/tools/`. Secrets come from config, never from tool arguments. |
| `src/storage/schema.rs` | About six schema versions | Move each migration's SQL into `storage/migrations/vN.sql`, loaded with `include_str!`. |
| New storage domain | Calendar cache, observations, enrollment | Its own `storage/<domain>/` module with its own types and SQL; the schema stays in `schema.rs`. |
| `src/app.rs` | `App::run` does more than wiring | Extract named startup helpers (`open_memory`, `start_speech`); `App` stays the composition root. |
| Second interface | Push-to-talk (roadmap 15A) | `src/interfaces/voice/` produces text for `control::parse` and renders `Notice`s; it shares nothing with the terminal module except `features/`. |

## Rust conventions

### Errors
- One `thiserror` enum per module boundary (`StorageError`, `MemoryError`, `EmbedError`, `AssistantError`, `ConfigError`). Derive `Display` and `Error`; don't hand-write them.
- Keep causes typed: use `#[source]` or `#[from]`, never `Variant(String)` or `.map_err(|e| e.to_string())`. Messages are lowercase with no trailing period.
- Fallible functions return `Result`. Never return `Option<SomeError>` to mean "maybe failed"; return `Result<(), E>`.
- `Box<dyn std::error::Error>` is only acceptable at the top of the binary (`main`, `App`, `run_input_loop`). Adding `anyhow` is a dependency change and needs approval.
- `expect("why this cannot fail")` is for broken invariants only. No `unwrap()` in `src/`. Never panic on input, I/O, model output, or a missing optional service.
- Error variants exist in every build. No `#[cfg(test)]` variants or fields in production types.

### Types and state
- IDs are newtypes (`SourceId`, `MemoryId`). Don't pass bare `i64` IDs across modules.
- Model state with enums (`EmbedderState`, `MemorySaveState`, `ImportState`). Never encode state in a string or test it with `.contains(...)` on a message.
- Prefer one owned value over clusters of `Option` fields and booleans that can disagree. If fields must change together, make them an enum.
- Match your own enums exhaustively; don't add a `_` arm that would hide a new variant.
- Use `let … else`, `if let` chains, and `matches!` where they read better. All are used in this codebase.
- For narrowing numeric conversions, prefer `try_from` or saturating arithmetic. Use `as` only where the range is already proven, and say why.

### Naming
- `snake_case` for modules, functions, fields and variables; `UpperCamelCase` for types and variants; `SCREAMING_SNAKE_CASE` for constants. Treat acronyms as words: `LocalLlm`, `TtsConfig`.
- Use `is_`, `has_`, `can_`, or `should_` for boolean queries; `as_` for cheap borrows; `to_` for conversions that allocate; `into_` for conversions that consume. No `get_` on plain getters.
- Name things for what they do now. Don't encode the parameter list in a method name (`append_turn`, not `append_turn_with_memory_and_sources`). Rename leftover vocabulary when a design changes (there are no "jobs" any more).
- Name a limit constant for its unit and meaning (`MAX_CHUNK_BYTES`, `RECALL_QUEUE_WAIT`). No magic numbers inline.

### Visibility and API shape
- Private by default. Use `pub(super)` for parent-only items and `pub(crate)` for cross-module items. Re-export a module's API from its `mod.rs` with `pub(crate) use`. Never widen visibility for tests.
- Borrow (`&str`, `&[T]`, `&Path`) unless ownership is needed. Clone `Arc`s or small values deliberately, not to silence the borrow checker.
- Take only the config you need: `LocalLlm::new(&LocalLlmConfig)`, not the whole `AppConfig`.

### Async and concurrency
- Blocking file or CPU work runs in `spawn_blocking`. SQLite goes through `tokio-rusqlite`'s connection handle. Model inference happens on the 16 GB Pi over HTTP, never in-process.
- Every external wait has a named timeout. A timeout means Jarvis stopped waiting. It does not mean remote or native work stopped, so never infer that it had no effect.
- Futures raced in `tokio::select!` must be safe to drop at any `.await`: commit durable state in single transactions, and keep in-flight native work owned (e.g., by a semaphore permit). Use `biased;` when user input must win.
- Use bounded channels. Use `Arc<Mutex<_>>` only for genuinely shared state, keep lock scopes short, and never hold a lock across `.await`.
- There is no `CancellationToken` or `JoinSet` supervision yet. Add one when a long-lived background task needs it (roadmap phase 6), not before.

### Unsafe
- `unsafe` is limited to FFI in `storage/sqlite_vec.rs`. Each block is minimal and carries a `// SAFETY:` comment stating the invariant. Don't add `unsafe` elsewhere without discussion.

### Comments and docs
- Use `///` on items used across modules, stating the invariant, limit, or why. Don't restate the code. Use `//!` for a module's purpose when it isn't obvious from the name.
- `// LEARNING:` comments are Darren's tutor notes. Keep the existing ones accurate when you change nearby code. Add new ones only in tutor mode.
- The system prompt lives in `src/features/assistant/preamble.txt`, not in Rust strings.

### Dependencies
- Don't add a dependency, crate, feature flag, or workspace without Darren's approval. Pin an exact, supported version and check that it builds for aarch64 Linux (the Pi).
- Keep `[dependencies]` sorted. Test-only features (such as `tokio/test-util`) go in `[dev-dependencies]`.
- Prefer the pinned libraries' supported APIs over custom loops, wrappers, or traits made "for flexibility". Rig owns the model loop, tool dispatch, and correlation; don't reimplement them.

## UI (TypeScript)

- The screen UI is a React, TypeScript and Vite app in `ui/` (spec: `docs/specs/jarvis-ui/spec.md`, ADR 0001). It holds no logic: it renders what Jarvis sends and reports taps.
- Run from `ui/`: `npm run build` (typecheck, Vite build to `ui/dist/`, then a check that `dist/index.html` has no root-absolute `src`/`href`), `npm run typecheck`, `npm run lint`, `npm test` (Vitest, tests in `ui/tests/`). `scripts/verify.sh` runs the last three; it runs `npm ci` only if `ui/node_modules` is missing.
- Every npm package needs Darren's approval and an exact version (no `^` or `~`; `ui/.npmrc` sets `save-exact`). Commit `package-lock.json`. `ui/dist/` and `ui/node_modules/` are git-ignored.
- Node runs on the Mac only. The Pi gets only the built `ui/dist/`.

## Domain rules

- **Policy.** `JarvisPolicyHook` enforces two model turns at most, one tool call at most, tool calls only on the first turn, and stops on truncated or unknown finish reasons. The second request still lists the tools, with tool choice `none`: dropping the list changes the start of the prompt and empties the server's prompt cache (fast-memory 10). Model request limits and tool attempt limits are separate; keep both.
- **Adding a tool.** Everything lives in `src/tools/`; `App` calls `tools::register` once.
  1. Implement `rig_agent::tool::Tool` with a typed `Args` struct carrying `#[serde(deny_unknown_fields)]`.
  2. Add `.tool(T)` to `tools::register`.
  3. Add `ToolRule::for_tool::<T>()` to `ALLOWED_TOOLS`. `registered_tools_match_the_allowed_list` fails if steps 2 and 3 disagree.
  4. A tool that writes or acts also needs per-tool permission and explicit user approval (roadmap 8B).
- **Trust.** Model text, tool arguments, recalled memory, and recognition results are untrusted observations. They never grant permission or establish identity.
- **Privacy.** Keep host and private data on the LAN. A cloud request needs explicit opt-in covering everything sent, including history and tool results. Never log or archive raw reasoning or tool-call JSON.
- **History.** Keep whole native Rig message batches. Append a batch only after a successful run, and evict whole batches. Never split a call from its result.
- **Run reports.** `RunObserver` is passive and fresh per run. Mark usage or timing that wasn't observed as `Unavailable`; never report an estimate or a zero as fact. Read structured lifecycle events, never stdout.

## Storage and memory

- SQLite (`data/jarvis.sqlite3`) is the only durable store. The FTS5 index (`memories_fts`) and sqlite-vec index (`memory_vectors`) live in the same file and are kept in sync by triggers. Don't add a second store that must be kept consistent with it.
- All SQL lives in `src/storage/`. Write paths use `TransactionBehavior::Immediate`. An archived turn and its memory source commit in one transaction.
- Memory text is immutable (a trigger enforces this). To change a memory, delete it and insert a new row. Forgetting deletes memory rows and marks sources `forgotten`, cascading through `source_links`. Archived `turns` are kept.
- Embeddings come from the BGE server on the 16 GB Pi through `clients/remote_embeddings.rs`, one request at a time. The server rejects input over 512 tokens, so the client tokenizes long text and keeps the first 511 tokens plus [SEP]. Any success marks the embedder ready again; idle embedding waits while it is unavailable.
- `memories.embedding_model` records which model made each vector. A new embedder fingerprint makes every row pending, and idle passes re-embed it. Vectors are 384-dimensional (`EMBEDDING_DIMENSIONS`).
- **Changing the schema:**
  1. Bump `SCHEMA_VERSION`.
  2. Add the next `version < N` migration step; it runs inside the existing initialization transaction.
  3. Never edit an old migration.
  4. Add a test in `tests/unit/storage/schema_tests.rs` that builds the previous version with realistic rows and asserts the migrated result.
  5. Tell Darren to back up the database before running it on the Pi.
- A new connection gets sqlite-vec through `sqlite_vec::register()` (already called by `ConversationArchive::open`).

## Testing

- **Layout.** `tests/unit/` mirrors `src/`. Tests for `src/a/b.rs` live in `tests/unit/a/b_tests.rs`; tests for `src/a/mod.rs` live in `tests/unit/a/a_tests.rs`. The source file registers them with exactly:
  ```rust
  #[cfg(test)]
  #[path = "<relative path>"]
  mod tests;
  ```
  No test code, helpers, or mocks in `src/`, and no visibility widened for tests.
- Workspace member crates under `crates/` keep their tests in `crates/<name>/tests/`; `tests/unit/` mirrors the root crate's `src/` only.
- **Integration tests** run the compiled binary and live in `tests/integration/`. Each is registered with a `[[test]]` entry in `Cargo.toml`. Never include unit tests in them.
- **Test limits at their boundaries.** For every limit, test the last accepted value and the first rejected one (2,048 vs 2,049 input bytes, 1,500 vs 1,501 chunk bytes, 32 vs 33 recalled sources, 383/384/385 dimensions). Group cases into a table instead of writing near-identical functions.
- **Test each rule once, in the layer that owns it.** The pure policy function covers every rule; hook tests cover only what the hook adds; HTTP tests cover only transport and wire format.
- **Use the production path.** Model tests always stream: script Rig mocks with `MockCompletionModel::from_stream_turns`, and have Wiremock return OpenAI-style SSE (`set_body_raw(body, "text/event-stream")`). Don't test through Rig's `.run()` or `.completion()`, and don't re-implement production logic inside a test helper.
- **Determinism.** No real LLM or external network. Use Wiremock on localhost, temporary SQLite files cleaned up by a `Drop` fixture, and `#[tokio::test(start_paused = true)]` instead of wall-clock sleeps.
- **Names** describe the behaviour, e.g. `a_turn_may_depend_on_at_most_32_recalled_sources`.
- **When moving tests,** compare `cargo test --locked --offline -- --list` before and after; the inventory changes only where you intend it to.
- **Spot-check a new boundary test** by temporarily breaking the limit by one and confirming it fails.

## Verification

While iterating, run the smallest relevant tests. Before saying a coherent change is done, run `scripts/verify.sh`, which runs this list for the whole workspace plus a check for banned patterns (`unwrap()`, `unsafe` outside `storage/sqlite_vec.rs`, drawn text):

```sh
cargo fmt --check
cargo check --locked --offline --all-targets
cargo test --locked --offline
cargo clippy --locked --offline --all-targets -- -D warnings
git diff --check
python3 -B tests/unit/features/speech/supertonic_worker/test_supertonic_worker.py  # if speech changed
```

Clippy stays at `-D warnings`. Report any check you skipped or couldn't run; never claim verification that didn't happen. Local tests do not prove Pi behaviour; say so when it matters.
