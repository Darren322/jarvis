# Jarvis repository guidance

Jarvis is a local-first voice assistant written in Rust 2024. It runs on a Raspberry Pi 5 (8 GB) and talks to a llama.cpp server on a second Pi (16 GB). Read [docs/ROADMAP.md](docs/ROADMAP.md) for current priorities and the principles that never change.

## Sources of truth

- The working tree, `Cargo.lock`, and `docs/` are the project facts. `notes/` (untracked) holds handoffs and the archived full roadmap: history, not rules.
- `rig-agent` and `rig-core` are pinned to exactly `0.42.0`. Check every library API against the locked version's source or docs (`~/.cargo/registry/src/`), not against memory, newer docs, or unversioned examples.

## Working with Darren

- For plans and material design choices, use the `grilling` skill to settle open decisions before acting. Settled decisions and explicit authorizations persist; don't ask again.
- Default learning workflow is hands-on Rust: explain one concept, offer one small patch with its rationale, then wait. Follow the requested scope; don't implement a whole phase when asked for a small change or guided learning, and don't start a phase just because it is documented.
- Preserve user changes. Don't commit, push, stage, or rewrite unrelated files unless asked. When asked to commit, make one commit per coherent step with a conventional prefix (`feat`, `fix`, `refactor`, `test`, `docs`, `build`, `chore`) and a body that says what changed and why.
- When delegating to sub-agents, give each explicit, non-overlapping file ownership, then integrate and review their work.
- Near a usage limit, write a factual handoff under `notes/`: completed work vs. proposals, changed files, decisions, verification evidence, source revision, and the exact next small action.

## Layout

| Path | Owns |
| --- | --- |
| `src/main.rs` | Tokio runtime, config load, `App`, bounded shutdown. Nothing else. |
| `src/app.rs` | Assembling clients and services; startup. No parsing, REPL flow, or domain logic. |
| `src/config.rs` | Environment parsing. Every path is resolved to absolute once, here. |
| `src/clients/` | Provider construction and transport (`local_llm`, `local_embeddings`). |
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
| `src/tools/` | About 8 tools, or measured wrong tool choices | Group by domain (`tools/calendar/{read,create}.rs`) and offer only the relevant tools per request via `RequestPatch::active_tools`. |
| Tool permissions | The first tool that writes or acts (roadmap 8B) | Add an access level to `ToolRule`. The approval prompt belongs to the interface, not the tool. |
| Tool clients | A tool needs the network | Transport in `src/clients/` with the `local_llm.rs` endpoint protections; a thin `Tool` wrapper in `src/tools/`. Secrets come from config, never from tool arguments. |
| `src/config.rs` | A third settings section (beyond LLM and TTS) | `config/mod.rs` plus one file per section (`config/tts.rs`, ...). |
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
- Native inference and blocking file or CPU work run in `spawn_blocking` (see `Embedder`). SQLite goes through `tokio-rusqlite`'s connection handle.
- Every external wait has a named timeout. A timeout means Jarvis stopped waiting. It does not mean remote or native work stopped, so never infer that it had no effect.
- Futures raced in `tokio::select!` must be safe to drop at any `.await`: commit durable state in single transactions, and keep in-flight native work owned (e.g., by a semaphore permit). Use `biased;` when user input must win.
- Use bounded channels. Use `Arc<Mutex<_>>` only for genuinely shared state, keep lock scopes short, and never hold a lock across `.await`.
- There is no `CancellationToken` or `JoinSet` supervision yet. Add one when a long-lived background task needs it (roadmap phase 6), not before.

### Unsafe
- `unsafe` is limited to FFI in `storage/sqlite_vec.rs` and `clients/local_embeddings.rs`. Each block is minimal and carries a `// SAFETY:` comment stating the invariant. Don't add `unsafe` elsewhere without discussion.

### Comments and docs
- Use `///` on items used across modules, stating the invariant, limit, or why. Don't restate the code. Use `//!` for a module's purpose when it isn't obvious from the name.
- `// LEARNING:` comments are Darren's tutor notes. Keep the existing ones accurate when you change nearby code. Add new ones only in tutor mode.
- The system prompt lives in `src/features/assistant/preamble.txt`, not in Rust strings.

### Dependencies
- Don't add a dependency, crate, feature flag, or workspace without Darren's approval. Pin an exact, supported version and check that it builds for aarch64 Linux (the Pi).
- Keep `[dependencies]` sorted. Test-only features (such as `tokio/test-util`) go in `[dev-dependencies]`.
- Prefer the pinned libraries' supported APIs over custom loops, wrappers, or traits made "for flexibility". Rig owns the model loop, tool dispatch, and correlation; don't reimplement them.

## Domain rules

- **Policy.** `JarvisPolicyHook` enforces two model turns at most, one tool call at most, tools only on the first turn, and stops on truncated or unknown finish reasons. Model request limits and tool attempt limits are separate; keep both.
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
- **Integration tests** run the compiled binary and live in `tests/integration/`. Each is registered with a `[[test]]` entry in `Cargo.toml`. Never include unit tests in them.
- **Test limits at their boundaries.** For every limit, test the last accepted value and the first rejected one (2,048 vs 2,049 input bytes, 1,500 vs 1,501 chunk bytes, 32 vs 33 recalled sources, 383/384/385 dimensions). Group cases into a table instead of writing near-identical functions.
- **Test each rule once, in the layer that owns it.** The pure policy function covers every rule; hook tests cover only what the hook adds; HTTP tests cover only transport and wire format.
- **Use the production path.** Model tests always stream: script Rig mocks with `MockCompletionModel::from_stream_turns`, and have Wiremock return OpenAI-style SSE (`set_body_raw(body, "text/event-stream")`). Don't test through Rig's `.run()` or `.completion()`, and don't re-implement production logic inside a test helper.
- **Determinism.** No real LLM or external network. Use Wiremock on localhost, temporary SQLite files cleaned up by a `Drop` fixture, and `#[tokio::test(start_paused = true)]` instead of wall-clock sleeps.
- **Names** describe the behaviour, e.g. `a_turn_may_depend_on_at_most_32_recalled_sources`.
- **When moving tests,** compare `cargo test --locked --offline -- --list` before and after; the inventory changes only where you intend it to.
- **Spot-check a new boundary test** by temporarily breaking the limit by one and confirming it fails.

## Verification

While iterating, run the smallest relevant tests. Before saying a coherent change is done, run:

```sh
cargo fmt --check
cargo check --locked --offline --all-targets
cargo test --locked --offline
cargo clippy --locked --offline --all-targets -- -D warnings
git diff --check
python3 -B tests/unit/features/speech/supertonic_worker/test_supertonic_worker.py  # if speech changed
```

Clippy stays at `-D warnings`. Report any check you skipped or couldn't run; never claim verification that didn't happen. Local tests do not prove Pi behaviour; say so when it matters.
