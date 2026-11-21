# 14: Note only real facts about Darren

Status: in-progress
Stories: US6
Blocked by: None
Assignee: coordinator (2026-11-21)
Owns: `src/features/assistant/note_facts.txt`, `src/features/memory/noting.rs` (the candidate check), and their tests
Traces to: D7

**Why:** On 2026-11-21 the idle noting pass stored, from Darren's test sentences and requests: "The user says facts.", "I say show my facts as a list.", "The user says \"forget that I say facts\".", "The user wants milk.", "The user wants eggs.", "The user wants bread.", "I want to show my facts as a list." (the last three from "Make me a shopping list: milk, eggs and bread."). Darren had them all deleted (backup `data/jarvis.sqlite3.bak-2026-11-21b`) and said "ok lets go with all".

**What to build:**
- The prompt names more of what is not a fact (requests and commands to Jarvis, what the user says about the words themselves, repeats and corrections of what was heard, wants and shopping lists) and gains NONE examples taken from the sentences above.
- A candidate is kept only if it is a sentence about the user, as the prompt asks: it starts with "The user" (case-insensitive). "I say show my facts as a list." is the model copying Darren's words, not a fact.

## Acceptance criteria
- [ ] Parse table: "The user ..." kept, "the user ..." kept, "I say ...", "I want ...", "User ..." and an empty fact dropped; the five-fact limit unchanged.
- [ ] `scripts/verify.sh` passes.
- [ ] Pi, on a database copy: the sentences above note nothing, and "My sister Mia lives in Penang." still notes a fact.

## Comments
