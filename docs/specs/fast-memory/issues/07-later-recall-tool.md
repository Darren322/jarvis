# 07: Later: a recall tool Gemma can call

Status: wontfix
Stories: None yet
Blocked by: 06
Assignee: unassigned
Owns: undecided

**What it would be:** A read-only tool the model calls to search memory when a question needs it and carries no trigger phrase. It costs a second model pass, and whether a small Gemma calls it at the right times is unmeasured. Out of scope for this epic by Darren's decision on 2026-11-14; decide after living with trigger phrases and the facts list. A tool that forgets is a write action and waits for roadmap step 8B.

## Comments
- 2026-11-21 (coordinator): closed as wontfix by Darren ("just remove it"). The facts list answers "what is my sister's name" with no trigger word, and trigger phrases plus `/recall` cover searches, so a model-called search tool would only add a second model pass. Reopen it if Jarvis misses questions that need a search and carry no trigger phrase.
