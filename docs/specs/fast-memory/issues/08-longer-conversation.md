# 08: Keep more of the conversation, and stop re-reading it every turn

Status: needs-info
Stories: US1
Blocked by: None
Assignee: unassigned
Owns: src/features/conversation/history.rs, tests/unit/features/conversation/history_tests.rs; the Gemma server's start command on the 16 GB Pi (needs Darren's OK)

**Why:** Darren asked on 2026-11-14 for more exchanges and a larger context window. Today `MAX_EXCHANGES = 3`: Jarvis forgets what was said four turns ago, and from the fifth turn on every turn drops the oldest exchange. That changes the start of the conversation Gemma has cached, so it re-reads all of it: `cached_input` falls to 275 (the system prompt alone) and the first word takes 1.5 to 1.9 s instead of about 0.4 s. Seen in Darren's voice log and in every benchmark run of 2026-11-14.

**What to build:**
1. **Trim in steps.** Let the history grow to an upper bound and then cut it back to a lower one, so the cached prefix changes once every several turns instead of every turn. Whole batches only (the AGENTS History rule).
2. **Bound history by what fits, not by a count of 3.** The budget must leave room for the system prompt (about 275 tokens), the facts list (about 300, ticket 03), a triggered search (about 250), the new prompt and the answer (`MAX_RESPONSE_TOKENS` is 1,024). Jarvis has no tokenizer for Gemma, so the bound is in bytes with a stated bytes-per-token margin, or in exchanges with a byte cap; decide with a measurement.
3. **Raise the Gemma server's context** (`-c 2048` today; each of its 4 slots has the full 2,048). The 16 GB Pi had about 10 GB of memory available on 2026-11-14. A Pi change: Darren's OK, a restart, and a note of the old and new command.

**Open before this can start (settle with Darren, by grilling):**
- How many exchanges to keep at most, and how far to cut back.
- The new context size (4,096 or 8,192), and whether to go to one slot (`--parallel 1`) at the same time.
- Whether `MAX_RESPONSE_TOKENS` stays at 1,024: it reserves half of today's context.

## Acceptance criteria
- [ ] Boundary tests for the trim: the last turn that does not trim and the first that does; after a trim exactly the lower bound remains; batches are never split.
- [ ] A streamed test showing two consecutive requests share the whole earlier history as a prefix between trims.
- [ ] The budget arithmetic is written down in the ticket with the measured figures.
- [ ] Pi acceptance with `scripts/pi/bench_first_text.py` on a conversation longer than the upper bound: `cached_input` grows turn by turn between trims, and the first word stays near 0.4 s on those turns.
- [ ] `scripts/verify.sh` passes.

## Comments
