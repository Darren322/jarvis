# 02: Say a filler line when a search starts

Status: ready-for-agent
Stories: US3
Blocked by: 01
Assignee: unassigned
Owns: src/interfaces/cli/repl/turn.rs, src/interfaces/cli/repl/mod.rs, src/interfaces/cli/repl/speech.rs, and their tests under tests/unit/interfaces/cli/repl/

**What to build:** Spec D4. When a prompt triggers a search, Jarvis at once prints one of the three canned lines and, when speech is on, speaks it as the first chunk of the answer's speech, before the model has produced anything. The lines rotate in order across searches. The line is not part of the answer: it is never archived, never added to the history and never sent to the model. Everything about stopping speech, taps and typed input during an answer stays as it is.

## Acceptance criteria
- [ ] A triggered search shows and speaks a filler line before the first model text; a plain prompt shows none. Tested with the scripted speech and model doubles.
- [ ] Three searches in a row use the three lines in order, then wrap.
- [ ] The archived turn and the next request's history contain no filler text.
- [ ] With speech off the line is still printed. `/stop`, a new prompt and a tap during the filler behave as during any spoken answer.
- [ ] The Verification list passes for the workspace.
- [ ] The report says what only the Pi proves (how the filler sounds before the answer, the gap between them).

## Comments
