# 04: The three timer tools and the access level on `ToolRule`

Status: ready-for-agent
Stories: US1, US2, US5, US6
Blocked by: 03
Assignee: unassigned
Owns: `src/tools/timers/**` (new: `mod.rs`, `set_timer.rs`, `cancel_timer.rs`, `list_timers.rs`, `resolve.rs`), `src/tools/mod.rs`, `src/app.rs` (only creating the service and passing the handle to `tools::register`; no other wiring), `tests/unit/tools/timers/**` (new), `tests/unit/tools/tools_tests.rs`, and, only if their expected tool lists must change to compile or pass, `tests/unit/clients/local_llm_tests.rs` and `tests/unit/features/assistant/*_tests.rs`. `AGENTS.md` is Darren's; propose the trigger-table wording in the Comment (D12).
Traces to: D2, D3, D4, D5, D10, D12

**What to build:**
- **Access level (D5):** `ToolRule` gains an access level (an enum, for example `ReadOnly`, `Acts`, and `Acts` meaning "needs approval"; the agent proposes the names). `set_timer` is `Acts`, `cancel_timer` and `list_timers` are not, and every existing tool keeps its current level. Nothing is enforced by the model: the approval itself is the confirmation in ticket 06.
- **`set_timer`** (typed `Args`, `deny_unknown_fields`, flat schema, no `anyOf`): optional `label`; exactly one of `minutes` or `at`. It only proposes: it calls the service's propose request (a `Pending` timer) and returns what was proposed with the resolved time, or the typed refusal for a 6th timer (D3). A label, minutes or `at` that is missing, both given, or out of range is a typed error whose message tells the model what to fix.
- **Time resolving (D2)** as a pure function of a `jiff::Zoned` and the request, in `resolve.rs`: minutes from now, or a clock time that is today if still ahead and tomorrow if not. Output includes the end instant and a spoken form ("11:00 pm tomorrow"; 12-hour as in `current_time`). `at` is 24-hour `"HH:MM"` (D18): the schema description tells the model to turn "11 pm" into `"23:00"`, and anything else is a typed error that says the expected form.
- **`cancel_timer`** (optional `label`) and **`list_timers`** (no arguments), by D10: cancel without confirmation, return the running list when the label matches zero or several timers.
- Register all three in `tools::register` and `ALLOWED_TOOLS`. All 8 tools stay offered on every request (D12; fast-memory 10).
- Tool descriptions name Darren's sentences (T1 to T5, E2) as the use cases and say the model must not claim a timer is set until the user confirms.

## Acceptance criteria
- [ ] The pure resolver is table-tested with fixed instants and zones: 25 minutes from now; "11 pm" at 10:59 pm (today) and at 11:00 pm and 11:01 pm (tomorrow); across midnight and a month end; a zone with a half-hour offset; a daylight-saving jump day. Minutes at the limits (the last accepted value and the first rejected one) are tested.
- [ ] Each tool's name, parameters schema (flat, `additionalProperties: false`, no `anyOf`/`oneOf`) and description are tested; `ToolRule` accepts valid arguments and rejects unknown keys, both `minutes` and `at`, and neither.
- [ ] `set_timer` creates only a `Pending` timer (checked against a fake or real service handle) and never a `Running` one; a 6th timer returns the refusal and creates nothing (5 accepted, 6 refused).
- [ ] Cancel and list behave as D10: no label with one timer, a matching label, no match and several matches (the last two return the list and cancel nothing).
- [ ] The access level is tested for every tool in `ALLOWED_TOOLS`: only `set_timer` needs approval.
- [ ] `registered_tools_match_the_allowed_list` passes with 8 tools; existing tests whose tool counts change are updated and listed in the Comment.
- [ ] The Comment records the proposed wording for AGENTS.md's "When to restructure" row on `src/tools/` (8 tools, every tool still offered, why).
- [ ] No `unwrap()`; `scripts/verify.sh` passes.
- [ ] Pi check (Darren's voice, after tickets 06 and 07 make it usable end to end): T1, T2, T3 (and E2) each lead Gemma to call `set_timer` with the right label and time, and T4 and T5 call `cancel_timer` and `list_timers`. Only the Pi shows whether Gemma picks the right tool and arguments.

## Comments
