# 09: A fresh conversation after 30 minutes of quiet

Status: ready-for-agent
Stories: none (added 2026-11-16 at Darren's request: "sure lets add 30 minutes of quiet")
Blocked by: None
Assignee: unassigned
Owns: src/features/conversation/history.rs, src/features/conversation/session.rs, src/interfaces/cli/repl/{mod,turn}.rs, and their tests under tests/unit/

**Why:** The history is only cut by size (ticket 08: 12 exchanges or 6 KiB). Nothing ends a conversation in time, so a question asked the next morning still carries the last exchanges of the night before. Darren also asked whether the Gemma prompt cache needs a time limit. It does not: a slot holds only the last prompt it read, it is reused only for a prompt that starts with the same tokens, and the saved states are capped by `--cache-ram` (checked read-only on the 16 GB Pi, 2026-11-16). The thing that should end in time is the conversation.

**What to build:**
- When a prompt (typed or spoken) is about to be sent and 30 minutes or more have passed since the last answer was recorded, Jarvis starts a new conversation first, then sends the prompt.
- No timer and no background task: the check runs when the prompt arrives. Nothing happens while Jarvis is quiet.
- The archive and the memories are untouched. A later "do you remember ..." still finds the old exchanges by search.

**Proposed by the coordinator, for Darren to overrule:**
- **Quiet is measured from the last recorded answer** (the end of the last successful turn), with Tokio's monotonic clock, so a change of the wall clock cannot trigger or block it. A failed turn does not restart the count.
- **One code path with `/reset`.** The quiet reset does to the conversation exactly what `/reset` does; it also stops a cache warm-up that is still running, as `/reset` does.
- **A named constant, not a setting:** `QUIET_BEFORE_NEW_CONVERSATION` = 30 minutes. A setting is added only if Darren wants to tune it on the Pi.
- **Jarvis says nothing about it.** One diagnostics line in the terminal ("new conversation after N minutes of quiet"); nothing is spoken and no sound plays.
- **Commands do not count.** `/memories`, `/help`, "close it" and a tap that only starts listening neither reset the conversation nor restart the count; only a prompt that reaches Gemma does.

## Acceptance criteria
- [ ] Boundary test with a paused clock: a prompt 1 ms before 30 minutes keeps the history; a prompt at exactly 30 minutes starts with an empty history.
- [ ] The first request after a quiet reset carries the system prompt and the new prompt only (streamed test, on the wire).
- [ ] Nothing is deleted: the archived turns and memories of the old conversation are still there after the reset.
- [ ] A failed turn does not restart the count; a command that never reaches Gemma neither resets nor restarts it.
- [ ] A warm-up still running when the reset happens is stopped. Run 20 times (`scripts/verify.sh --repeat`).
- [ ] The very first prompt after start-up does not reset anything and prints no diagnostics line.
- [ ] `scripts/verify.sh` passes.

## Comments
