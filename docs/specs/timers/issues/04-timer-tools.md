# 04: The three timer tools and the access level on `ToolRule`

Status: done
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
- [x] The pure resolver is table-tested with fixed instants and zones: 25 minutes from now; "11 pm" at 10:59 pm (today) and at 11:00 pm and 11:01 pm (tomorrow); across midnight and a month end; a zone with a half-hour offset; a daylight-saving jump day. Minutes at the limits (the last accepted value and the first rejected one) are tested.
- [x] Each tool's name, parameters schema (flat, `additionalProperties: false`, no `anyOf`/`oneOf`) and description are tested; `ToolRule` accepts valid arguments and rejects unknown keys, both `minutes` and `at`, and neither.
- [x] `set_timer` creates only a `Pending` timer (checked against a fake or real service handle) and never a `Running` one; a 6th timer returns the refusal and creates nothing (5 accepted, 6 refused).
- [x] Cancel and list behave as D10: no label with one timer, a matching label, no match and several matches (the last two return the list and cancel nothing).
- [x] The access level is tested for every tool in `ALLOWED_TOOLS`: only `set_timer` needs approval.
- [x] `registered_tools_match_the_allowed_list` passes with 8 tools; existing tests whose tool counts change are updated and listed in the Comment.
- [x] The Comment records the proposed wording for AGENTS.md's "When to restructure" row on `src/tools/` (8 tools, every tool still offered, why).
- [x] Once `app.rs` creates the service, the markers in `src/storage/timers/mod.rs` (`#![expect(dead_code)]`) and on `ConversationArchive::timers_repository` in `src/storage/archive.rs` come off (both files join this ticket for that only; the build forces it, as both expectations become unfulfilled). The `features/timers` marker stays: `confirm`, `reject`, `has_pending` (ticket 06), `watch_shown` (07) and the `TimerEvent` fields (08) are still unused; whichever ticket leaves nothing dead there removes it.
- [x] `set_timer` refuses a label over `orb_protocol::MAX_TIMER_LABEL_CHARS` (60 characters) before calling `propose`, tested at 60 and 61: one stored long label would make ticket 07's `Timer::new` fail and the whole `timers` list unsendable.
- [x] No `unwrap()`; `scripts/verify.sh` passes.
- [ ] Pi check (Darren's voice, after tickets 06 and 07 make it usable end to end): T1, T2, T3 (and E2) each lead Gemma to call `set_timer` with the right label and time, and T4 and T5 call `cancel_timer` and `list_timers`. Only the Pi shows whether Gemma picks the right tool and arguments.

## Comments
- 2026-11-22 (coordinator): built by a Sonnet agent in a worktree from `f53de7c`; reviewed and integrated into `dev`.
  - **New files:** `src/tools/timers/` holds `set_timer`, `cancel_timer`, `list_timers`, `resolve.rs` and `TimerToolError`. Constants: `MIN_TIMER_MINUTES = 1` and `MAX_TIMER_MINUTES = 1440`. `set_timer`'s `Args` take exactly one of `minutes` or `at`, enforced at deserialization, so `ToolRule` rejects both and neither. The `at` format, the minutes range and the label length (60 characters, checked before `propose`) are typed errors the model reads. Output `status: proposed_not_set_yet`, with a note never to say the timer is set.
  - **Resolver:** it builds the civil time in the zone and never adds 24 h. A clock time equal to now means tomorrow. Its table covers Singapore, Kolkata, and the New York spring-forward and fall-back days.
  - **Access level:** `ToolAccess::{NoApproval, NeedsApproval}`, set by `.needing_approval()`. `ToolRule::access` keeps a `cfg_attr(not(test), expect(dead_code))` marker until ticket 06 reads it.
  - **Wiring:** `app.rs` runs the service as one more `select!` branch beside the panel inbox. The event receiver is kept alive for tickets 06 and 08. Two changes went outside the ticket's files because the code would not compile otherwise, and the coordinator accepted both: `ConversationSession::timers_repository()`, and the extra `channel().0` argument at every test call site of `tools::register`. Tool counts in the existing tests went from 5 to 8. The storage dead-code markers are gone. AGENTS.md's `src/tools/` and Tool permissions rows were updated with the agent's wording.
  - **Integration fix by the coordinator:** D23 (`0e7454a`) landed after the agent started. Three `set_timer` tests proposed several timers and waited for them all to be pending, so they hung. They now fill the limit with running timers (`run_timer`) and wait only for the newest pending one.
  - **Evidence:** spot-checks (agent): 1440 to 1441 failed 3 tests, and the label `>` to `>=` failed 1. Coordinator: `scripts/verify.sh` in the main checkout, all checks passed (701 tests in the main crate, 460 Vitest tests). The Pi check (Gemma's choice of tool and arguments for T1 to T5 and E2) waits for tickets 06 and 07.
- 2026-11-26, first try on the Pi (coordinator):
  - **What happened:** Darren said "set a timer for five seconds". Gemma called no tool and answered "I have set a timer for five seconds", and no timer was created. While that answer stayed in the conversation, a copy of Jarvis with his database answered "2 minutes" and "10 minutes" the same way, again without calling the tool. On an empty database "3 minutes" was proposed, and `list_timers` found it.
  - **Fix:** `set_timer` gains a `seconds` field. A whole number of minutes in range is proposed. Anything else gets `ResolveError::NotWholeMinutes`, whose message tells the model to say the shortest timer is 1 minute. The description now says to call the tool for every timer, however short or long.
  - **Tests:** the `seconds` boundary table in `resolve_tests.rs` (60, 120 and 86,400 accepted; 0, 1, 5, 59, 61 and 90 refused; 86,460 out of range), argument cases in `set_timer_tests.rs`, and a tool test where 5 s is refused without creating a timer and 60 s is proposed.
