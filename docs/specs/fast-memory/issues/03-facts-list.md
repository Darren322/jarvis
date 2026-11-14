# 03: The facts list, stored and sent with every request

Status: ready-for-agent
Stories: US4
Blocked by: 01
Assignee: unassigned
Owns: src/storage/** for a new facts module and the schema step, src/features/memory/** for a facts API, src/features/assistant/** and src/features/conversation/** for the request placement, and their tests under tests/unit/

**What to build:** Spec D5 and D8. A facts record apart from the searchable memories: text, where it came from (stated by Darren or noted by Jarvis) and when. It needs a schema step: follow "Changing the schema" in AGENTS.md (bump the version, one migration step, a migration test, and tell Darren to back up the Pi database first). All facts go into every request as one message directly after the system prompt, in a stable order and wording so the request prefix is byte-identical while the list is unchanged. A byte cap (about 1,200 bytes for the list, and a per-fact limit) is enforced on insert, with the D8 rule: noted facts are dropped oldest-first to make room; stated facts are never dropped automatically, and when they alone fill the list the insert is refused with a typed error. Facts are never returned by a search and never recalled twice. No command or model pass adds facts yet: tickets 04 and 05 do.

## Acceptance criteria
- [ ] Migration test from the previous schema version with realistic rows; old data intact.
- [ ] Insert, list and remove, all SQL in `src/storage/`, write paths in immediate transactions.
- [ ] Boundary tests: the list cap and one byte more; the per-fact limit and one byte more; a full list of stated facts refuses a new one; a noted fact is dropped for a new one, oldest first.
- [ ] Streamed model test: the facts message sits directly after the system prompt; two runs with the same list send an identical prefix; an empty list adds no message.
- [ ] The Verification list passes for the workspace.
- [ ] The report says what only the Pi proves (that `cached_input` covers the facts on the second request).

## Comments
