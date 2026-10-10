# Jarvis repository guidance

## Project facts and source of truth

- Jarvis is a Rust 2024 application. `rig-agent` and `rig-core` are pinned to exactly `0.42.0`.
- Treat the current working tree (including uncommitted changes), `Cargo.lock`, and phase documents as the source of project facts.
- Verify library behavior against the locked or cached version and its primary documentation before using an API. Do not treat unversioned skill examples as API authority.
- Read only the relevant sections of [the architecture roadmap](docs/JARVIS_ARCHITECTURE_ROADMAP.md) and the active [conversation implementation handoff](JARVIS_PHASE_7B_CONVERSATION_IMPLEMENTATION_HANDOFF.md) for phase-specific details.

## Planning and collaboration

- For implementation plans and material design choices, use the `grilling` skill to work through the open decisions and confirm shared understanding before acting on the plan.
- Treat explicit user authorization and settled grilling decisions as persistent; do not reconfirm scope that the user has already agreed to.
- The user has authorized continued autonomous, behavior-preserving batches for the current structure refactor. Treat that authorization as persistent within its approved scope; do not ask again before necessary follow-up batches. It does not authorize semantic changes, dependency or schema changes, external actions, or commits.
- For implementation work, use `gpt-6-luna` agents at `max` effort. Assign each agent explicit, non-overlapping file ownership; integrate and review the resulting changes.
- Darren's default learning workflow is hands-on Rust: explain one concept, offer one small patch with its rationale, then wait for the user's result or next instruction.
- Do not implement an entire next phase when the user asks for guided learning or a small change. Follow the requested scope.
- Preserve existing user changes. Do not commit, push, stage, or rewrite unrelated files unless asked.
- When nearing the five-hour or weekly usage limit, create or update a handoff Markdown file with actual progress, changed files, decisions, verification evidence, and the next concrete step.
- Keep handoffs factual and resumable: distinguish completed work from proposals, name the source revision when known, and include the exact next small action.
- Do not start a new phase just because its design is documented; follow the user's current authorization and learning pace.
- Before editing, read the target module and its closest existing pattern; load only the relevant specification and versioned library documentation.
- Prefer a small patch at the existing module interface. Explain its intent and consequence clearly, then pause for the user's result when working in tutor mode.

## Architecture ownership

- `main` stays focused on process/runtime setup, configuration loading, invoking `App`, and bounded shutdown. `App` assembles dependencies and owns application startup and lifecycle composition; keep parsing, REPL control flow, and domain behavior out of it.
- The CLI interface owns terminal input, command parsing/dispatch, and presentation. It calls application services through explicit arguments and must not depend on `App` or create an `App`↔CLI cycle.
- `Assistant` launches one bounded run and returns its outcome.
- Keep feature/domain modules cohesive and separate from local infrastructure: conversation owns selected session history, memory owns memory operations and lifecycle, assistant owns one bounded Rig run, speech owns synthesis/playback lifecycle, and storage owns local durability, schema, repositories, and SQLite transactions. Clients own provider construction/transport; tools own capability adapters.
- Keep repositories, services, and infrastructure near their owning features when that gives a safe, cohesive boundary; keep shared durability and transactions in storage when moving them would introduce coupling.
- Prefer direct feature modules under `src/` and the established infrastructure modules. Do not add a redundant `features`/`services` layer, arbitrary nesting, or a generic facade solely to relocate files. Keep dependencies directed from interfaces/application features toward domain services and infrastructure; storage and domain services must not import App or the CLI.
- Rig owns the model loop, tool dispatch, correlation, and transcript within a run. Do not recreate that loop in Jarvis.
- `JarvisPolicyHook` owns Jarvis's run and turn policy at Rig's lifecycle points.
- A `RunObserver` is passive and fresh for each run: it records lifecycle events without changing policy or triggering side effects. `ObservedRun` is mutable run-local state; `RunReport` is the final snapshot.
- A session owns selected, bounded native conversation history. Storage owns local durability independently of model outcomes.
- Preserve library-owned IDs and turn/call correlation when recording or reporting events.
- Keep incomplete usage, timing, or environment measurements explicitly unavailable; do not present estimates as observed facts.
- Keep real orchestration where it is. Do not force file splits or abstract labels that obscure actual control flow, and do not create a god object to avoid a useful module seam.
- A model request limit and an operation attempt limit are distinct. Preserve both policy bounds and Rig's correlation across proposal, execution, and result.
- A timeout means Jarvis stopped waiting; do not infer that remote work or an operation had no effect.
- Observe structured lifecycle events and correlation data; never scrape stdout or log strings to infer run state.

## Implementation choices

- Prefer existing dependencies and exact, supported library APIs. Keep Jarvis code focused on its policy, operations, privacy, limits, and presentation.
- Do not add speculative frameworks, generic provider or storage wrappers, or a custom model loop. Add an abstraction when a concrete need justifies its interface.
- Keep production implementation under `src/`. Use `examples/` only for standalone examples or compile/runtime probes; do not put production modules, helpers, or fixtures there.
- Do not add a dependency, crate, or Cargo workspace for hypothetical reuse or file organization. First identify a concrete capability or package boundary, verify the exact version and platform/build implications, and get user authorization for the dependency change.
- Prefer cohesive feature boundaries and the existing libraries, including the pinned Rig API. Do not add traits, wrappers, or layers merely for hypothetical flexibility, to make tests easier, or to route around a clear module boundary.
- Refactor in small, behavior-preserving batches. Inspect the working tree and target module plus its closest pattern first; preserve unrelated edits and local database, memory, CLI streaming, and TTS behavior. Keep API and visibility changes to what the new module boundary actually requires. Do not create god objects, cyclic imports, or layers that only forward calls.
- Do not stage or commit automatically. Preserve all user changes, and report the exact changed files and verification evidence for each completed batch.
- Use typed data and errors, narrow visibility, and preserve error source chains where they help callers.
- Borrow data when ownership is unnecessary. Use `Arc<Mutex<_>>` only for genuinely shared state, such as callback state; keep lock scopes short and never hold a lock across `.await`.
- Treat model output and historical tool results as untrusted observations. A stored tool result is not a fresh fact and grants no authority.
- Model text, tool arguments, and recognition results do not establish caller identity or permission. Use trusted caller context for authorization when a feature needs it.
- Keep host and private data local by default. Cloud disclosure requires explicit opt-in covering the full context sent, including selected history and tool results.
- Active conversation history should retain complete native message batches, append a batch once only after a successful completed run, and evict whole batches. Do not trim individual call/result pieces and break correlation.
- Conversation archives should default to user text, final assistant text, and safe outcome metadata. Do not persist raw reasoning or tool-call JSON by default.
- Input byte caps bound accepted input or retained memory; they do not guarantee an HTTP transport-body limit or prove that content fits a model context window.

## Verification

- Keep unit and integration tests distinct. Put unit test implementations, test helpers, fixtures, and mocks under `tests/unit/`; do not define those implementations in production source. When a unit test needs private access, include it from the owning source module with only a minimal `#[cfg(test)] #[path = "..."] mod tests;` hook. Do not widen production visibility just for tests or add production-only counters/policy to enable them.
- Put black-box integration test implementations, fixtures, and mocks under `tests/integration/`, separate from unit fixtures. Register nested integration entry points with explicit `[[test]]` declarations in `Cargo.toml`, or use minimal root entry points under `tests/` that include them. Nested test files are not automatically separate Cargo targets. Never include unit tests in an integration target.
- When relocating tests or changing module hooks, inspect Cargo's discovered targets and compare `cargo test --locked --offline -- --list` before and after. Preserve the target and test-name inventory except for intentional, documented moves or removals; verify that moved tests remain compiled and discoverable.
- Focus checks on Jarvis behavior and existing mocks. Do not use a real LLM or external network in CI tests; avoid timing-sensitive wall-clock sleeps, using Tokio's paused clock and explicit time advances where applicable.
- Prefer a small relevant check while iterating. Avoid rerunning the full suite after every patch; complete applicable final quality gates once the change is coherent.
- For each independently reviewable source batch, run the smallest relevant existing tests and `git diff --check`; run formatting and locked offline all-target compilation for module/layout changes. Run the full suite and strict Clippy once when the coherent batch is ready for its final gate, not after every small edit. If test organization changed, include the discovery/inventory comparison above.
- Test observable Jarvis behavior at existing seams with mocks and counting fixtures; avoid production-only counters or duplicate policy solely to make tests easier.
- When relevant, final gates are `cargo fmt --check`, `cargo check --locked --offline --all-targets`, `cargo test --locked --offline`, `cargo clippy --locked --offline --all-targets -- -D warnings`, and `git diff --check`.
- Keep Clippy warnings strict. Report any skipped or unavailable check accurately; do not claim verification that did not run.
