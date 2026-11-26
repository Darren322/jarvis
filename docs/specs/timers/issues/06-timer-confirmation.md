# 06: Confirming a timer: "yes" and "no", and the fixed lines

Status: done
Stories: US2, US5
Blocked by: 03, 04
Assignee: unassigned
Owns: `src/features/control/**` (`command.rs`, a new `timer.rs`, `mod.rs`), `src/interfaces/cli/repl/**`, `src/interfaces/cli/input.rs`, `src/interfaces/cli/presentation.rs`, `src/app.rs` (only passing the timers handle to the REPL), `tests/unit/features/control/**`, `tests/unit/interfaces/cli/**`
Traces to: D4, D10, D11

**What to build:** The approval prompt belongs to the interface (AGENTS.md). Voice and the CLI:
- **Parse:** `control::parse` understands "yes" and "no" (and close variants the agent proposes from Darren's usage, T6, such as "yes please" or "no thanks") only while a timer is pending. When none is pending they go to the model as usual. Anything else said while a timer is pending goes to the model and the 30-second window keeps running (D4). How `parse` learns that a timer is pending is the agent's design to propose (open question: the spec does not say whether `parse` gains a flag or the REPL checks `has_pending` first); stop and report with the proposal before building.
- **One pending timer at most (D23):** the service replaces a pending timer on every new proposal, so "yes" and "no" act on the only pending one (`TimersHandle::shown()` holds it).
- **Handling:** a `Command` for yes and one for no, handled in `features/control/` and returning `Notice`s that the REPL prints and speaks; they call the service's confirm and reject. They never reach the model, so the model cannot confirm anything.
- **Fixed lines (Jarvis's own text, not model output):** after a run in which `set_timer` proposed a timer, Jarvis speaks **only** the fixed line; the model's own reply is still printed and kept in the history unchanged (D16). After `set_timer` proposes: "Set 'Time to sleep' for 11:00 pm tomorrow? Say yes or tap Set." (an unlabelled timer says "Set a timer for 10 minutes?" style text; the agent writes it and Darren judges it on the Pi); on confirm: "Set."-style acknowledgement with the resolved time; on drop (no, or the window running out): "Dropped the timer."; after a cancel by voice: "Cancelled 'Time to sleep'."; after a cancel from the screen (D11): a fixed line, with no model run. The wording lives in one place with the other fixed lines.
- **Window running out and screen taps:** the service's events (ticket 03) are turned into spoken or printed notices while idle, read only in the REPL's idle `select!`, like the announcements of jarvis-ui 11 (C6). Taps on Set, Don't, Cancel and Keep arrive in ticket 07 and call the same handlers.

## Acceptance criteria
- [x] `parse` table: "yes", "no" and the proposed variants are commands only while a timer is pending, and are plain text to the model otherwise; the same sentence with extra words ("yes I think the sky is blue") is not a confirmation.
- [ ] While a timer is pending, an unrelated sentence goes to the model and the pending timer is still there; the window is not extended or reset by it.
- [x] Confirm at 30 s sets the timer; "yes" just past 30 s (30 s plus 1 ms) is plain text to the model and sets nothing (tested with the paused clock).
- [x] "yes" and "no" produce the fixed lines and never start a model run (a test checks that no request is made).
- [ ] After a run that proposed a timer, only the fixed line is spoken; the model's reply is printed and its batch is in the history unchanged (D16).
- [ ] The window running out produces the drop line once, while idle; if Jarvis is busy it waits for idle.
- [x] Timing and `select!` tests pass `scripts/verify.sh --repeat <pattern>` (20 runs).
- [x] No new command is added without a `COMMANDS` help row where it belongs, or a recorded reason why not.
- [x] No `unwrap()`; `scripts/verify.sh` passes.
- [ ] Pi check, timing: record the seconds from the proposal (inside the run) to Darren's "yes" being parsed, over three tries of T1; if any is over about 20 s, the 30 s window (D4) goes back to Darren.
- [ ] Pi check (Darren's voice): T1 then "yes" sets the timer; T2 then "no" drops it; T6 after the window runs out drops it with the line. (Fully usable once ticket 07 lets him tap too.)

## Comments
- 2026-11-22 (coordinator): implemented by the coordinator in the main checkout (streaming speech and cross-module wiring).
  - **Parse:** `Command::TimerAnswer { answer, text }` is parsed for a whole "yes" or "no" and close variants (`YES_PHRASES`, `NO_PHRASES`), ignoring case and punctuation. "yes I think the sky is blue" stays a prompt. The REPL answers it through `control::answer_timer`; with no timer pending, or its window over, the same text runs as a `Prompt`. This settles the ticket's open question: `parse` stays pure and the handler decides. `COMMANDS` gains a "yes / no" row.
  - **Fixed lines:** in `features/control/timer.rs`, every one takes `now`: `proposal_line`, `set_line`, `REJECTED_LINE`, `expired_line`, and `event_notice` for the idle events. Until 08, an ended timer is only announced. `TimerLink` (handle and events) is the REPL's one new parameter. Timer events are read last in the idle `select!`; a `Failed` one is logged, not spoken.
  - **D16:** `turn.rs` `Proposal` notes the pending timer at the start of a run. When a different pending timer appears (the run's own proposal), the fixed line is fed to speech once and the model's text stops being spoken. The text is still printed and archived, and the line is printed after the reply, also when the run fails or is landed after a tap.
  - **Tests:** the parse rows; 6 `control::timer` tests (the lines at a fixed instant; yes, no, nothing pending; 30 000 vs 30 001 ms against a real service on a set clock); 3 `Proposal` tests; and a REPL test where "Yes." sets a pending timer with no model request and the next "yes" goes to the model. Evidence: `scripts/verify.sh --repeat timer` (all checks passed, 711 tests in the main crate, 20 of 20).
  - **Not yet shown by a test** (so left unticked): an unrelated sentence leaving the pending timer and its window alone (true by construction, since a prompt never calls the service); the D16 swap inside a full streamed run with speech; the drop line through the REPL after a real window runs out (only its line and the idle-only read are tested). The voice cancel's "Cancelled" line comes from the model's reply to `cancel_timer`; the screen cancel's fixed line comes with 07.
- 2026-11-26: closed by Darren after trying it on the Pi ("the feat works"). Seen in the Pi log at 13:52 to 13:54: a timer proposed by voice, then "Yes.", answered "Set. The timer ends at 1:54 pm today." Not checked on their own: "no" dropping a pending timer, the window running out, and the proposal-to-yes timing.
