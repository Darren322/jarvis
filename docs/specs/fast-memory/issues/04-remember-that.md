# 04: "Remember that…" saves a fact

Status: done
Stories: US5
Blocked by: 03
Assignee: Sonnet agent (2026-11-21)
Owns: src/features/control/**, src/interfaces/cli/repl/mod.rs, and their tests under tests/unit/

**What to build:** Spec D6. A line that starts with "remember that" (case-insensitive, with the punctuation and "please" handling the forget phrases already use) or `/remember <fact>` saves the rest as a stated fact, in the words given, with no model run. Jarvis answers "Got it." as a notice and, when speech is on, speaks it. A refused save (list full, fact too long, nothing after the phrase) says why in plain words. It is a command, so it works typed or spoken, is never sent to the model and is never a search. The saved fact becomes the "forget that" focus.

## Acceptance criteria
- [x] Parse table: the phrase forms that save, near-misses that stay prompts ("remember when we…" is a search, not a save), `/remember` with and without text.
- [x] A save reaches the facts list, makes no model request and no search, and replies "Got it."; each refusal has its message.
- [x] "forget that" directly after a save removes that fact.
- [x] `/help` lists the command. The Verification list passes for the workspace.

## Comments

2026-11-21 (coordinator): built by a Sonnet agent in a worktree (in parallel with 05); the coordinator read the whole source diff, integrated it into `dev`, changed the too-long message from "Keep it under 200 characters." to "Keep it to 200 characters or fewer." (200 is accepted), and ran `scripts/verify.sh` in the main checkout: all checks passed. Committed at Darren's standing OK; not yet deployed.
- **Parsing** (`control/command.rs`): `Command::Remember(String)`, a `/remember <fact>` row in `COMMANDS`; "remember that <fact>" after any of "hey jarvis", "jarvis", "okay", "ok", "please" (comma-tolerant). Prompts, not saves: "remember that time/day/night/one…" (whole words; "timeout" still saves), "remember when…", "remembering…", "remember thatcher…", "do you remember that…", "I remember that…". `/remember` or "remember that" alone shows the usage line. The existing forget phrases still do not skip "hey Jarvis"; left as they were.
- **Saving** (`MemoryControl::remember`): a stated fact through `FactsRepository::insert`; the focus becomes `Focus::Fact`, so "forget that" removes it (even with no memory service). A refused save clears the focus. Messages: "Got it."; "To make room I forgot: …."; the usage line; "That is too long for me to remember. Keep it to 200 characters or fewer." (the limit is bytes); "I can only remember one line at a time."; "I can't remember more: the list is full of things you told me. Forget one first."; "I can't save that: local memory storage is unavailable."; "Forgot that: <text>"; "That fact is already gone; nothing was forgotten."
- **Speaking:** the REPL prints each line as a notice and, with speech on, says them through `speak_lines` (`repl/speech.rs`: `SpokenAnswer`, `feed_line`, `finish_text`). It waits for the few words to play; input typed meanwhile is read after. The orb stays idle.
- **Tests:** about 30 parse rows plus `/help`; REPL tests for a save with no model request and no state, the spoken "Got it.", a refused save, and "remember that time…" staying a prompt; `MemoryControl` tests for every refusal (200 vs 201 bytes), dropped noted facts, no storage, and "forget that" after a save. Spot-check by the agent: dropping "one" from `PAST_WORDS` failed the parse table; restored.
- **Lint:** the `expect(dead_code)` on `ConversationSession::facts_repository` was removed (now used).
- **Only the Pi proves:** a spoken "remember that…" through speech-to-text, and how the spoken "Got it." sounds.
- 2026-11-21 (coordinator): deployed to the 8 GB Pi with 03 to 06 at `e2bc1a8` (Darren: "yes build restart etc"). Built while Jarvis ran (45.9 s); stopped it at 16:28:35, backed up the database with `sqlite3 .backup` to `data/jarvis.sqlite3.bak-2026-11-21` (integrity check ok; version 4, 366 turns, 344 memories), started it. After start, read-only: `user_version` 6, 366 turns and 344 memories, 0 facts, all 366 turns `facts_examined = 1`; the screen reconnected. Waiting for Darren's voice checks.
- 2026-11-21 (coordinator): closed by Darren after his voice check of `5b16e6a` ("voice check of 5b16e6a is good").
