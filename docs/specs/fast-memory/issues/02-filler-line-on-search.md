# 02: Say a filler line when a search starts

Status: done
Stories: US3
Blocked by: 01
Assignee: Claude (Sonnet agent)
Owns: src/interfaces/cli/repl/turn.rs, src/interfaces/cli/repl/mod.rs, src/interfaces/cli/repl/speech.rs, and their tests under tests/unit/interfaces/cli/repl/

**What to build:** Spec D4. When a prompt triggers a search, Jarvis at once prints one of the three canned lines and, when speech is on, speaks it as the first chunk of the answer's speech, before the model has produced anything. The lines rotate in order across searches. The line is not part of the answer: it is never archived, never added to the history and never sent to the model. Everything about stopping speech, taps and typed input during an answer stays as it is.

## Acceptance criteria
- [x] A triggered search shows and speaks a filler line before the first model text; a plain prompt shows none. Tested with the scripted speech and model doubles.
- [x] Three searches in a row use the three lines in order, then wrap.
- [x] The archived turn and the next request's history contain no filler text.
- [x] With speech off the line is still printed. `/stop`, a new prompt and a tap during the filler behave as during any spoken answer.
- [x] The Verification list passes for the workspace.
- [x] The report says what only the Pi proves (how the filler sounds before the answer, the gap between them).

## Comments

2026-11-14 (Claude): implemented by a Sonnet agent, reviewed and re-verified by Claude, committed and pushed with Darren's approval.
- **Built.** `FILLER_LINES` in `repl/turn.rs` ("Let me check.", "One moment.", "Let me look that up."); `PromptRequest.filler`; the input loop keeps the rotation (`next_filler`) and advances it once per searching run. `run_prompt` shows the line as a notice right after `Thinking` and, when the answer is spoken, queues it as the first speech chunk through the new `ChunkFeed::feed_line`. It is never sent to the model, archived or added to the history.
- **Verified by Claude on the Mac:** read the whole source diff; the Verification list is clean three runs in a row (347 root unit tests, 5 new). The agent ran the five new tests 10 times without a failure.
- **Gap:** `/stop` during a searching answer is tested; a tap or a new typed line during the filler has no test of its own and relies on the filler being an ordinary first chunk.
- **Side effect:** "time to first audio" now usually measures the filler, not the answer's first word.
- **Only the Pi proves:** how the filler sounds ahead of the answer and the gap between them. Not run there yet.
