# 06: List and forget facts

Status: ready-for-agent
Stories: US7
Blocked by: 04, 05
Assignee: unassigned
Owns: src/features/control/**, and their tests under tests/unit/features/control/

**What to build:** Spec D9. `/memory facts` lists the facts with numbers and whether each was stated or noted. `/memory forget <number>` removes one. "forget that" directly after a "Noted:" notice removes that fact (the forget focus already covers the last answer and a single listed memory; add the last noted or saved fact). "forget <detail>" also considers facts, and still proceeds only for one unique match across memories and facts. All of it through the existing parser and `MemoryControl`, returning `Notice`s; typed or spoken; no model.

## Acceptance criteria
- [ ] Listing shows numbers, text and origin; an empty list says so.
- [ ] Each removal path removes exactly the intended fact; an ambiguous "forget <detail>" removes nothing and says why.
- [ ] A removed fact is absent from the next request.
- [ ] `/help` lists the commands. The Verification list passes for the workspace.

## Comments
