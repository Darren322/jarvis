# 06: List and forget facts

Status: in-review
Stories: US7
Blocked by: 04, 05
Assignee: Sonnet agent (2026-11-21)
Owns: src/features/control/**, src/interfaces/cli/repl/idle.rs and the idle-read region of src/interfaces/cli/repl/mod.rs (so a "Noted:" fact becomes the "forget that" focus; widened by the coordinator on 2026-11-21), the dead-code `expect` attributes in src/storage/facts.rs and src/storage/mod.rs, and their tests under tests/unit/

**What to build:** Spec D9. `/memory facts` lists the facts with numbers and whether each was stated or noted. `/memory forget <number>` removes one. "forget that" directly after a "Noted:" notice removes that fact (the forget focus already covers the last answer and a single listed memory; add the last noted or saved fact). "forget <detail>" also considers facts, and still proceeds only for one unique match across memories and facts. All of it through the existing parser and `MemoryControl`, returning `Notice`s; typed or spoken; no model.

## Acceptance criteria
- [x] Listing shows numbers, text and origin; an empty list says so.
- [x] Each removal path removes exactly the intended fact; an ambiguous "forget <detail>" removes nothing and says why.
- [x] A removed fact is absent from the next request.
- [x] `/help` lists the commands. The Verification list passes for the workspace.

## Comments

2026-11-21 (coordinator): built by a Sonnet agent in the main checkout; the coordinator read the diff, replaced the agent's `#[cfg(test)]` re-export in `storage/mod.rs` (test-only code in `src/`) with a compile-time assert in `features/memory/facts.rs` (`FACT_BULLET.len() + 1 == FACT_LINE_OVERHEAD_BYTES`) plus `super::` imports in the storage tests and literal figures in the memory test, and ran `scripts/verify.sh`: all checks passed. Committed at Darren's standing OK.
- **Commands:** `/memory facts` lists `<id> [stated|noted] <text>` (the number is the stable `FactId`; empty: "I have no saved facts."). `/memory forget <number>`: "Forgot that: <text>", or "There is no fact number <n>; nothing was forgotten.", or the usage line "Use /memory forget followed by a fact number from /memory facts. (Memories use /forget <id>.)". Both work with no memory service.
- **"forget that":** after a save (04) or a "Noted:" line removes that fact. The idle read sets `last_noted` (an out-parameter, so a tap that ends the wait keeps it), and the loop calls `MemoryControl::focus_on_noted_fact` before handling the next wake.
- **"forget <detail>":** case-insensitive substring over all facts and the existing memory matches; proceeds only for exactly one match across both. Several: "That detail matches more than one memory or fact. Use /memories to choose a memory ID or /memory facts to choose a fact number; nothing was forgotten." None: "I could not find one memory or fact matching that detail; nothing was forgotten."
- **Tests:** listing and empty list, a number that stays valid after an earlier removal, removal by number and a bad number, "forget that" after a noted fact, detail matching (unique fact, several facts, a memory and a fact, none, no memory service), focus moving on, and two REPL tests that a removed fact is absent from the next model request. `/help` lists both commands. Spot-check by the agent: letting several matching facts remove the first failed its test; restored.
- **Changed expectation:** the ambiguous-detail message now says "more than one memory or fact".
- **Lint:** all the `expect(dead_code/unused_imports)` attributes from 03 are gone.
- **Not covered:** one test running a real idle "Noted:" pass then a typed "forget that" (the REPL harness has no memory service); the two wiring lines in `repl/mod.rs` are covered by reading only.
