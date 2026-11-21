# 11: Warm the cache after a fact is noted

Status: needs-triage
Stories: US4, US6
Blocked by: 05
Assignee: unassigned
Owns: undecided (likely `src/interfaces/cli/repl/idle.rs`, `src/interfaces/cli/repl/mod.rs`, `src/features/conversation/session.rs`)

**Why:** Found in 05's review (2026-11-21). The facts message sits directly after the system prompt (D5), so a fact noted while idle changes the prompt from that point and the next answer rereads the whole conversation (the cost fast-memory 08 and 10 removed). D5 accepted this for rare changes; noted facts make it more frequent. The warm-up sent after a trim also carries the facts read at the start of that turn, so after a new fact it warms a stale prefix.

**Proposal (not settled):** after an idle pass that stored a fact, send a cache warm-up with the new facts message and the current history, in the rest of the idle time, cancelled by input like every warm-up. Measure first on the Pi how long the reread takes after a new fact; build only if it is noticeable.

## Acceptance criteria
- [ ] Pi measurement: the prompt reread after a noted fact, with and without the warm-up.

## Comments
- 2026-11-21 (coordinator): read-only look at `journalctl --user-unit jarvis` on the 8 GB Pi for a turn right after a noted fact. **No clean measurement yet.** The only noted fact today ("The user has a sister named Mia.", 17:05:31) was followed by a Jarvis restart (the `390e71d` deploy at 17:14) before the next model turn, so that turn began a new conversation; it read 993 tokens with 970 cached (first text 0.61 s). The forget at 17:16:48 was followed by the `5b16e6a` restart, which also changed the system prompt: that next turn (17:24:01) had 332 of 1,032 tokens cached and waited 12.5 s for the first text, which shows what a reread of about 700 tokens costs but is not a facts-only change. **Estimate, not measured:** a changed facts list makes Gemma reread everything after it (the facts, the history and the new prompt). With today's short conversations (input about 1,100 to 1,165 tokens, of which about 950 is the system prompt, tools and facts) that is about 200 tokens, roughly 3 to 4 s at the measured 58 tokens a second; near the 6 KiB history cap it would be well over 10 s. To measure it cleanly: either wait for a real noted fact in a running conversation and read the next `Model stage:` line, or run `scripts/pi/bench_first_text.py` with `--pause` on a database copy (this sends requests to Gemma, so it needs Darren's OK). Decision for Darren at triage: build the warm-up now, or wait for the clean number.
