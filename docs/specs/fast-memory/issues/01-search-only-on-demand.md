# 01: Search memory only on demand

Status: in-progress
Stories: US1, US2
Blocked by: None
Assignee: Claude (Sonnet agent, high effort)
Owns: src/features/memory/recall.rs, src/features/memory/mod.rs, src/features/conversation/session.rs, src/features/control/command.rs, src/features/control/mod.rs, src/interfaces/cli/repl/mod.rs, src/interfaces/cli/repl/turn.rs, and their tests under tests/unit/

**What to build:** Stop recalling on every prompt (spec D1 to D3). Add one pure function that says whether a prompt asks for a memory search: true for a prompt containing a trigger phrase of D2 as whole words, case-insensitive. Add the `/recall <question>` command (a variant, a `parse` arm and a `COMMANDS` row), which runs `<question>` as a prompt with a search forced. A run without a search sends no memory message and prints no memory warning: "not asked for" is its own case, distinct from "memory unavailable". Shrink a search to at most 2 memories and 1,024 bytes (named constants). Recall's ranking and relevance rules are otherwise unchanged, and so is what gets archived. A line that starts with "remember that" is reserved for ticket 04: until then it is an ordinary prompt and must not trigger a search.

## Acceptance criteria
- [ ] The trigger decision is one pure function, tested as a table: every phrase of D2, mixed case, a phrase inside a longer word, "remember that …" (no search), and plain prompts ("Hello", "How's the weather?").
- [ ] Streamed model tests: a plain prompt's request carries no memory message even when matching memories exist; a trigger prompt's and a `/recall` prompt's request carries it, placed as before.
- [ ] Boundary tests: 2 memories kept and a 3rd dropped; 1,024 bytes accepted and 1,025 rejected.
- [ ] A run without a search prints no memory warning and still archives and indexes the turn.
- [ ] `/recall` with no question prints a usage hint; `/help` lists it.
- [ ] `cargo fmt --all --check`, `cargo check`, `cargo test` and `cargo clippy -- -D warnings` with `--locked --offline --workspace`, and `git diff --check`, pass.
- [ ] Pi acceptance: the five-question measurement of 2026-11-14 repeated against a copy of the database; the report gives the figures.

## Comments
