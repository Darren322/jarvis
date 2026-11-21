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
