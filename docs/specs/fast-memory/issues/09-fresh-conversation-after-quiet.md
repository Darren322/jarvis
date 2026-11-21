# 09: A fresh conversation after 30 minutes of quiet

Status: in-review
Stories: none (added 2026-11-16 at Darren's request: "sure lets add 30 minutes of quiet")
Blocked by: None
Assignee: Sonnet agent (2026-11-21)
Owns: src/features/conversation/history.rs, src/features/conversation/session.rs, src/interfaces/cli/repl/{mod,turn}.rs, and their tests under tests/unit/

**Why:** The history is only cut by size (ticket 08: 12 exchanges or 6 KiB). Nothing ends a conversation in time, so a question asked the next morning still carries the last exchanges of the night before. Darren also asked whether the Gemma prompt cache needs a time limit. It does not: a slot holds only the last prompt it read, it is reused only for a prompt that starts with the same tokens, and the saved states are capped by `--cache-ram` (checked read-only on the 16 GB Pi, 2026-11-16). The thing that should end in time is the conversation.

**What to build:**
- When a prompt (typed or spoken) is about to be sent and 30 minutes or more have passed since the last answer was recorded, Jarvis starts a new conversation first, then sends the prompt.
- No timer and no background task: the check runs when the prompt arrives. Nothing happens while Jarvis is quiet.
- The archive and the memories are untouched. A later "do you remember ..." still finds the old exchanges by search.

**Proposed by the coordinator, approved by Darren on 2026-11-21 ("Okay sure!"):**
- **Quiet is measured from the last recorded answer** (the end of the last successful turn), with Tokio's monotonic clock, so a change of the wall clock cannot trigger or block it. A failed turn does not restart the count.
- **One code path with `/reset`.** The quiet reset does to the conversation exactly what `/reset` does; it also stops a cache warm-up that is still running, as `/reset` does.
- **A named constant, not a setting:** `QUIET_BEFORE_NEW_CONVERSATION` = 30 minutes. A setting is added only if Darren wants to tune it on the Pi.
- **Jarvis says nothing about it.** One diagnostics line in the terminal ("new conversation after N minutes of quiet"); nothing is spoken and no sound plays.
- **Commands do not count.** `/memories`, `/help`, "close it" and a tap that only starts listening neither reset the conversation nor restart the count; only a prompt that reaches Gemma does.

## Acceptance criteria
- [x] Boundary test with a paused clock: a prompt 1 ms before 30 minutes keeps the history; a prompt at exactly 30 minutes starts with an empty history.
- [x] The first request after a quiet reset carries the system prompt and the new prompt only (streamed test, on the wire).
- [x] Nothing is deleted: the archived turns and memories of the old conversation are still there after the reset.
- [x] A failed turn does not restart the count; a command that never reaches Gemma neither resets nor restarts it.
- [x] A warm-up still running when the reset happens is stopped. Run 20 times (`scripts/verify.sh --repeat`).
- [x] The very first prompt after start-up does not reset anything and prints no diagnostics line.
- [x] `scripts/verify.sh` passes.

## Comments

2026-11-21 (coordinator): built by a Sonnet agent in a worktree; the coordinator read the whole diff and integrated it uncommitted into `dev`, changing only the diagnostics line to "New conversation after N minutes of quiet." to match the other terminal lines.
- **Design.** `ConversationSession` keeps `last_answer_at` (`tokio::time::Instant`), set only when a run succeeds and cleared by `reset`; `quiet_for()` reports the gap. The REPL checks it in the `Prompt | Recall` arm before `run_prompt`, against `QUIET_BEFORE_NEW_CONVERSATION` (30 minutes, private to `repl/mod.rs`). `/reset` and the quiet reset share `reset_conversation` (warm-up cancel, `session.reset()`, `memory_control.clear()`); `/reset` keeps its speech stop and its "Conversation context reset." line.
- **Tests** (7, all in `tests/unit/interfaces/cli/repl/repl_tests.rs`, paused clock): 30 min minus 1 ms keeps the history and exactly 30 min empties it (spot-checked both ways: `>` for `>=` and the limit lowered by 1 ms each failed); the request after a quiet reset carries only the new prompt and the same preamble; turns, sources and memories of the old conversation are still in SQLite; the first prompt after start-up resets nothing after 2 hours; a failed turn does not restart the count; `/help`, `/memories` and "close it" neither reset nor restart it; `reset_conversation` drops a warm-up request still in flight.
- **Verified.** `scripts/verify.sh` in the main checkout: all checks passed. `scripts/verify.sh --repeat quiet` and `--repeat ending_the_conversation_stops`: 20 of 20 each.
- **Differs from the criteria's wording.** "On the wire" is checked on the requests the streaming Rig mock received, as the other REPL tests do, not with Wiremock: the HTTP layer's own tests cover the wire format. The diagnostics line is not asserted (the REPL's stderr is not captured in tests).
- **Not verified.** The Pi: a real 30-minute gap with Jarvis running there.
- 2026-11-21 (coordinator): committed as `cf112ae` and pushed at Darren's request ("just deploy it"). Deployed to the 8 GB Pi: Jarvis idle since 14:09, `git pull` to `cf112ae`, `cargo build --release --locked` (42.4 s), `systemctl --user restart jarvis` at 14:26:42; the service is active and the screen reconnected (one WebSocket client on 8765). The 30-minute reset has not yet been seen there.
