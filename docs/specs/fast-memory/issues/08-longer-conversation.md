# 08: Keep more of the conversation, and stop re-reading it every turn

Status: done
Stories: US1
Blocked by: None
Assignee: Claude
Owns: src/features/conversation/history.rs, src/features/conversation/session.rs, src/features/assistant/mod.rs, src/interfaces/cli/repl/{mod,turn,warmup}.rs and their tests under tests/unit/; scripts/pi/bench_first_text.py (a `--pause` option); the Gemma server's start command on the 16 GB Pi (Darren's OK given)

**Why:** Darren asked on 2026-11-14 for more exchanges and a larger context window. Today `MAX_EXCHANGES = 3`: Jarvis forgets what was said four turns ago, and from the fifth turn on every turn drops the oldest exchange. That changes the start of the conversation Gemma has cached, so it re-reads all of it: `cached_input` falls to 275 (the system prompt alone) and the first word takes 1.5 to 1.9 s instead of about 0.4 s. Seen in Darren's voice log and in every benchmark run of 2026-11-14.

**What to build:**
1. **Trim in steps.** Let the history grow to an upper bound and then cut it back to a lower one, so the cached prefix changes once every several turns instead of every turn. Whole batches only (the AGENTS History rule).
2. **Bound history by what fits, not by a count of 3.** The budget must leave room for the system prompt (about 275 tokens), the facts list (about 300, ticket 03), a triggered search (about 250), the new prompt and the answer (`MAX_RESPONSE_TOKENS` is 1,024). Jarvis has no tokenizer for Gemma, so the bound is in bytes with a stated bytes-per-token margin, or in exchanges with a byte cap; decide with a measurement.
3. **Raise the Gemma server's context** (`-c 2048` today; each of its 4 slots has the full 2,048). The 16 GB Pi had about 10 GB of memory available on 2026-11-14. A Pi change: Darren's OK, a restart, and a note of the old and new command.

**Settled with Darren by grilling, 2026-11-15:**
- **Bounds.** History grows to 12 exchanges and is cut back to 6.
- **Byte cap.** `MAX_HISTORY_BYTES` goes from 16 KiB to 6 KiB, measured as today (the stored messages as JSON). Hitting it is a trim like any other: cut back to 6 exchanges, then drop the oldest until the history is under 3 KiB. Confirm the figure against Gemma's tokenizer during the build.
- **Warm the cache after a trim.** As soon as the turn that caused the trim is recorded, Jarvis sends the trimmed history to Gemma with a one-token limit, so it overlaps the spoken answer. A prompt or tap drops the warm-up and goes ahead. A failed or timed-out warm-up gives a diagnostics line only. It is never archived, added to history or spoken. This widens `Owns:` beyond `history.rs`; the coordinator implements the ticket.
- **Gemma server.** `-c 4096` and `--parallel 2` (one slot for the conversation, one for ticket 05's idle pass) in `jarvis-gemma.service` on the 16 GB Pi. Darren's OK is given; restart at the Pi acceptance step, tell him just before, and keep a dated backup of the service file.
- **`MAX_RESPONSE_TOKENS` stays at 1,024.**

**Budget (4,096 tokens):** system prompt 275, facts list 300, a triggered search 250, the new prompt about 60, the answer 1,024; about 2,190 left for history.

**Measured 2026-11-15 (8 GB Pi, read-only, 175 completed turns; bytes of prompt plus answer):** all turns: median 72, mean 129, 90th percentile 251, largest 1,164. Last 100: median 98, mean 171, 90th percentile 302. The longest answer is 1,089 bytes. At an estimated 4 bytes a token plus about 10 tokens of chat markers, a recent exchange is about 55 tokens, 12 are about 660, and the 6 kept after a trim are about 330 (about 6 s of reading at 58 tokens a second). The bytes-per-token figure is an estimate, not yet checked against the tokenizer.

**16 GB Pi as seen 2026-11-15:** `jarvis-gemma.service` runs `llama-server --offline -hf unsloth/gemma-4-E2B-it-qat-GGUF:UD-Q4_K_XL --spec-type draft-mtp --spec-draft-n-max 3 -t 4 -c 2048 -b 512 -ub 128 --jinja --host 0.0.0.0 --port 8080 --temp 0 --reasoning off --swa-full`; `/props` reports 4 slots with `n_ctx` 2,048; 10.3 GB of memory available.

## Acceptance criteria
- [x] Boundary tests for the trim: the last turn that does not trim and the first that does; after a trim exactly the lower bound remains; batches are never split.
- [x] Boundary tests for the byte cap: exactly 6 KiB is kept and one byte more trims; a byte trim stops at exactly 3 KiB and drops one more exchange at one byte over.
- [x] A streamed test showing two consecutive requests share the whole earlier history as a prefix between trims.
- [x] The warm-up is the exact start of the next request on the wire (messages and tools), asks for one token, stops waiting at its deadline, and is never archived or added to history.
- [x] A trimming turn starts one warm-up; the next model run stops it; ending the loop stops it. Run 20 times.
- [x] The budget arithmetic is written down in the ticket with the measured figures.
- [x] Pi acceptance with `scripts/pi/bench_first_text.py` on a conversation longer than the upper bound: `cached_input` grows turn by turn between trims, and the first word stays near 0.4 s on those turns.
- [x] The Gemma server runs with 4,096 tokens a slot and 2 slots, checked with `/slots`, and the benchmark is repeated against it.
- [x] `scripts/verify.sh` passes.

## Comments

2026-11-15 (Claude): implemented by Claude in the main checkout; uncommitted. The Gemma server change is not made yet: Jarvis was running on the 8 GB Pi, and Darren asked to be told just before a restart.
- **Built.** `History` grows to `MAX_EXCHANGES` (12) and a full history is cut back in one step to `TRIMMED_EXCHANGES` (6), then further while it is over `TRIMMED_HISTORY_BYTES` (3 KiB); `MAX_HISTORY_BYTES` is 6 KiB. `record_success` returns `Recorded { Kept, Trimmed, Cleared }`. After a trim the session hands back a `CacheWarmup` (every stored message up to the last exchange's prompt) in `ConversationTurn::cache_warmup`. `Assistant::warm_cache` sends it with a one-token limit and a 30 s deadline, with the same system prompt and tools as a real run. The REPL's `WarmupTask` (`repl/warmup.rs`) runs it as a spawned task that is aborted when the next model run starts, on `/reset` and when the loop ends.
- **Two differences from the grilling answers.** (1) The warm-up is not dropped by a tap or a typed line as such, only when the next prompt is sent to Gemma, so it also runs through listening and speech-to-text. (2) A request has to end with a prompt, so the warm-up stops at the last exchange's prompt; the last answer (and the new prompt) are still read on the next turn.
- **Tokenizer check (Gemma's `/apply-template` and `/tokenize`, read-only, ten windows of 12 real exchanges).** 29 to 124 tokens an exchange, 47 on average. Text is 2.4 to 4.2 bytes a token; the JSON that `MAX_HISTORY_BYTES` counts is 5.2 to 6.7 bytes a token, because of about 110 bytes of JSON around each exchange. So 6 KiB is at most about 1,200 tokens for exchanges like Darren's and under 2,048 even at 3 bytes a token: inside the 2,190 budget. One of the ten windows (7.7 KB) would have trimmed on bytes before reaching 12 exchanges.
- **Verified on the Mac.** `scripts/verify.sh`: all checks passed (root unit tests 347 to 353). `scripts/verify.sh --repeat warm_up`: 20 of 20. Spot-checks: `MAX_EXCHANGES` 13, `TRIMMED_EXCHANGES` 7, `<` for `<=` at the byte cap and `>=` for `>` at the trim target each failed at least one test.
- **Pi acceptance (8 GB Pi, release build in a scratch worktree, a copy of the database, Gemma still at 2,048 tokens and 4 slots, 20 short questions).** With 12 s between questions: first word 0.28 to 0.71 s on every turn, 9.3 s in total; `cached_input` grows on each turn up to the trim (558 of 574 on turn 13), and on turn 14, the first after the trim, it is 407 of 427 with the first word at 0.43 s. With the questions back to back, so the warm-up is cut off at once: turn 14 reads 148 new tokens (`cached_input` 275) and waits 2.40 s; every other turn 0.32 to 0.53 s. That is the cost without a warm-up, once in seven turns instead of on every turn.
- **Not verified.** The same by voice. A conversation near the byte cap (the benchmark's exchanges are about 22 tokens). Anything against a 4,096-token slot.
- **Planned server command.** `-c 8192 --parallel 2`, not `-c 4096 --parallel 2`: `llama-server --help` on the Pi says the shared context pool is on only while the slot count is automatic, so with an explicit `--parallel 2` the context is divided between the slots, and 8,192 gives each slot 4,096.

2026-11-15 (Claude), later: the Gemma server change is made, with Darren's go.
- **Changed on the 16 GB Pi.** `/etc/systemd/system/jarvis-gemma.service` and the copy in the home directory: `-c 2048` became `-c 8192 --parallel 2`; nothing else in the command changed. The old unit file is `~/jarvis-gemma.service.bak-2026-11-15`. After `daemon-reload` and a restart the server was healthy in about 8 s; `/slots` reports 2 slots with `n_ctx` 4,096 each; 10.3 GB of memory still available.
- **Benchmark repeated (same build, same 20 questions, 12 s apart).** Turns 2 to 20: first word 0.38 to 0.71 s; `cached_input` grows up to the trim (557 of 573 on turn 13) and is 407 of 427 on turn 14 with the first word at 0.44 s. The total was 13.4 s because the first turn after a server restart reads the system prompt uncached (about 4 s).
- **A prompt larger than the old limit is accepted:** a direct request of 3,705 prompt tokens with a one-token limit returned normally.
- **Not verified.** The same by voice, and a real conversation that reaches the byte cap.
- **Cleaned up.** Everything this ticket put in `/tmp` on the 8 GB Pi is removed, and the scratch worktree is gone. The Pi checkout is still at `52fdd18`, so the Jarvis running there does not have this change until it is committed, pushed and pulled.

2026-11-15 (Claude): committed as `b2dd2be` and pushed at Darren's request. The 8 GB Pi checkout is at `b2dd2be` with debug and release builds; the Jarvis already running there keeps the old build until Darren restarts `./run.sh`.
