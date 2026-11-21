# 13: Warm the prompt cache when Jarvis starts

Status: in-progress
Stories: US1
Blocked by: None
Assignee: coordinator (2026-11-21)
Owns: `src/features/conversation/session.rs` (the start-up warm-up), `src/features/assistant/mod.rs` (`WARMUP_DEADLINE`), `src/interfaces/cli/repl/mod.rs` (start it before the first read), `src/app.rs` (wiring), and their tests
Traces to: D5, ticket 08, ticket 10

**Why:** Found on the Pi on 2026-11-21 (jarvis-ui 12). The start of every request (system prompt, tools and facts) is now about 1,600 tokens, and Gemma reads about 58 tokens a second, so after the cache is emptied the first question takes about 28 to 32 s against the 30 s `RUN_DEADLINE`. It failed once ("The streamed model response failed.") and passed once at 28.8 s. The cache is emptied by a change to the prompt or tools (every deploy that changes them), by a Gemma restart, and when the other slot's prompts push it out. Darren: "ok lets go with all".

**What to build:** when the REPL starts, before the first read, start a cache warm-up of a fresh conversation's first request: the system prompt, the tools, the facts message and a fixed stand-in prompt; the server caches everything up to where the real prompt will differ. It runs through the existing `WarmupTask`, so the first model run cancels it. `WARMUP_DEADLINE` goes from 30 s to 60 s: a warm-up keeps nobody waiting, and a cold read of the start is close to 30 s.

## Acceptance criteria
- [ ] REPL test: with a start-up warm-up, the first model request is the warm-up (only the facts message and the stand-in prompt, one-token answer), and a typed prompt still runs and is answered.
- [ ] The stand-in prompt and the deadline are named constants.
- [ ] `scripts/verify.sh` passes; a timing or cancellation test passes `--repeat` (20 runs).
- [ ] Pi: after a restart, the first question's `cached_input` covers the start (about 1,600 tokens) and its first word comes in about a second.

## Comments
