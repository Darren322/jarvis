# 13: Warm the prompt cache when Jarvis starts

Status: done
Stories: US1
Blocked by: None
Assignee: coordinator (2026-11-21)
Owns: `src/features/conversation/session.rs` (the start-up warm-up), `src/features/assistant/mod.rs` (`WARMUP_DEADLINE`), `src/interfaces/cli/repl/mod.rs` (start it before the first read), `src/app.rs` (wiring), and their tests
Traces to: D5, ticket 08, ticket 10

**Why:** Found on the Pi on 2026-11-21 (jarvis-ui 12). The start of every request (system prompt, tools and facts) is now about 1,600 tokens, and Gemma reads about 58 tokens a second, so after the cache is emptied the first question takes about 28 to 32 s against the 30 s `RUN_DEADLINE`. It failed once ("The streamed model response failed.") and passed once at 28.8 s. The cache is emptied by a change to the prompt or tools (every deploy that changes them), by a Gemma restart, and when the other slot's prompts push it out. Darren: "ok lets go with all".

**What to build:** when the REPL starts, before the first read, start a cache warm-up of a fresh conversation's first request: the system prompt, the tools, the facts message and a fixed stand-in prompt; the server caches everything up to where the real prompt will differ. It runs through the existing `WarmupTask`, so the first model run cancels it. `WARMUP_DEADLINE` goes from 30 s to 60 s: a warm-up keeps nobody waiting, and a cold read of the start is close to 30 s.

## Acceptance criteria
- [x] REPL test: with a start-up warm-up, the first model request is the warm-up (only the facts message and the stand-in prompt, one-token answer), and a typed prompt still runs and is answered.
- [x] The stand-in prompt and the deadline are named constants.
- [x] `scripts/verify.sh` passes; a timing or cancellation test passes `--repeat` (20 runs).
- [x] Pi: after a restart, the first question's `cached_input` covers the start (about 1,600 tokens) and its first word comes in about a second.

## Comments
- 2026-11-21 (coordinator): built in the main checkout; committed as `c43215f`, with `2ef4000` for a regression it caused: the binary test of `/help`, `/reset` and `/exit` saw the warm-up's POST whenever it reached the mock before `/exit` (it had passed once by timing). That test now allows at most the one-token warm-up and forbids any answer request; `scripts/verify.sh --repeat startup_warmup` and `--repeat binary_runs_help` 20 of 20 each. `STARTUP_WARMUP_PROMPT` ("Hello.") stays private to the session (the REPL test checks the warm-up's shape: one user message, `max_tokens` 1). Spot-check: the start removed, the REPL test failed; restored. Deployed at 21:02:57 and 21:09:50: Gemma's log shows the warm-up arriving at start (LCP 0.995, 7 new tokens: the start was already cached that time); on a database copy the first question had 1,521 of 1,542 tokens cached and its first word at 0.55 s. Not yet seen: a start after a Gemma restart, when the cache is really cold. Closed at Darren's word ("close the ticket after u are done").
