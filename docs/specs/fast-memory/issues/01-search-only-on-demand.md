# 01: Search memory only on demand

Status: done
Stories: US1, US2
Blocked by: None
Assignee: Claude (Sonnet agent, high effort)
Owns: src/features/memory/recall.rs, src/features/memory/mod.rs, src/features/conversation/session.rs, src/features/control/command.rs, src/features/control/mod.rs, src/interfaces/cli/repl/mod.rs, src/interfaces/cli/repl/turn.rs, and their tests under tests/unit/

**What to build:** Stop recalling on every prompt (spec D1 to D3). Add one pure function that says whether a prompt asks for a memory search: true for a prompt containing a trigger phrase of D2 as whole words, case-insensitive. Add the `/recall <question>` command (a variant, a `parse` arm and a `COMMANDS` row), which runs `<question>` as a prompt with a search forced. A run without a search sends no memory message and prints no memory warning: "not asked for" is its own case, distinct from "memory unavailable". Shrink a search to at most 2 memories and 1,024 bytes (named constants). Recall's ranking and relevance rules are otherwise unchanged, and so is what gets archived. A line that starts with "remember that" is reserved for ticket 04: until then it is an ordinary prompt and must not trigger a search.

## Acceptance criteria
- [x] The trigger decision is one pure function, tested as a table: every phrase of D2, mixed case, a phrase inside a longer word, "remember that …" (no search), and plain prompts ("Hello", "How's the weather?").
- [x] Streamed model tests: a plain prompt's request carries no memory message even when matching memories exist; a trigger prompt's and a `/recall` prompt's request carries it, placed as before.
- [x] Boundary tests: 2 memories kept and a 3rd dropped; 1,024 bytes accepted and 1,025 rejected.
- [x] A run without a search prints no memory warning and still archives and indexes the turn.
- [x] `/recall` with no question prints a usage hint; `/help` lists it.
- [x] `cargo fmt --all --check`, `cargo check`, `cargo test` and `cargo clippy -- -D warnings` with `--locked --offline --workspace`, and `git diff --check`, pass.
- [x] Pi acceptance: the five-question measurement of 2026-11-14 repeated against a copy of the database; the report gives the figures.

## Comments

2026-11-14 (Claude): implemented by a Sonnet agent at high effort, reviewed and re-verified by Claude, committed as `ce581c7` and pushed; the Pi checkout is at `ce581c7` and builds in debug and release.
- **Built.** `RecallMode { Search, Skip }` and `wants_memory_search(prompt)` in `features/memory/recall.rs`; `Command::Recall` for `/recall <question>`; the REPL chooses the mode (`recall_mode`) and hands it to `run_prompt` in a `PromptRequest`, and the session skips recall entirely for `Skip`: no embedding request, no query, no memory message, no warning. A search now hands over at most 2 memories and 1,024 bytes.
- **Matching rule.** The prompt is split into lowercase words on anything that is not a letter or digit; a phrase matches as consecutive whole words. A line that starts with "remember that" (after an optional "hey" and "jarvis") is reserved for saving a fact and does not search, except when the next word is "time", "day", "night" or "one". That exception was added during the work because Darren's own example of a search was "remember that time…"; it is in spec D6.
- **Verified by Claude on the Mac:** read the source diff; `cargo fmt --all --check`, `check`, `clippy -D warnings` and `test` with `--locked --offline --workspace`, and `git diff --check`: clean. Root unit tests 337 to 342. Spot-check by Claude: removing "time" from the exception failed the trigger table. The agent reports the 2 vs 3 memories and 1,024 vs 1,025 bytes boundaries each failed when broken by one.
- **Evidence for the problem (Pi, 2026-11-14, five typed questions, a copy of Darren's database, wait for the first word):** before: 6.00, 8.18, 1.42, 15.60 and 1.69 s, total 32.9 s; with an empty memory: total 3.1 s. Trial builds: 2 memories and 1,024 bytes 9.7 s; the same with keyword matches held to the relevance floor 9.1 s; 1 memory and 600 bytes 8.6 s.
- **Pi acceptance (same five questions, this ticket's build in a scratch worktree):** 0.29, 0.44, 0.39, 0.46 and 1.50 s, total 3.1 s. Two searches: "Do you remember what we said about the weather?" 3.0 s and `/recall what time did I ask about` 3.9 s, both answered from memory.
- **Not verified:** the same by voice on the real Pi run (voice adds the 1 s silence wait and about 2 s of speech-to-text).
- **Open, not this ticket:** the fifth question waits 1.5 s in every run, old and new, because the model's cache falls back to the system prompt on that turn (`cached_input` 275). It is not recall and not history trimming (the history cap is 16 KB). Unexplained; the Gemma server's four slots on a 2,048-token context are the first suspect.
