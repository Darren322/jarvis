# 04: "Remember that…" saves a fact

Status: ready-for-agent
Stories: US5
Blocked by: 03
Assignee: unassigned
Owns: src/features/control/**, src/interfaces/cli/repl/mod.rs, and their tests under tests/unit/

**What to build:** Spec D6. A line that starts with "remember that" (case-insensitive, with the punctuation and "please" handling the forget phrases already use) or `/remember <fact>` saves the rest as a stated fact, in the words given, with no model run. Jarvis answers "Got it." as a notice and, when speech is on, speaks it. A refused save (list full, fact too long, nothing after the phrase) says why in plain words. It is a command, so it works typed or spoken, is never sent to the model and is never a search. The saved fact becomes the "forget that" focus.

## Acceptance criteria
- [ ] Parse table: the phrase forms that save, near-misses that stay prompts ("remember when we…" is a search, not a save), `/remember` with and without text.
- [ ] A save reaches the facts list, makes no model request and no search, and replies "Got it."; each refusal has its message.
- [ ] "forget that" directly after a save removes that fact.
- [ ] `/help` lists the command. The Verification list passes for the workspace.

## Comments
