# 15: Bring up the stats without the model

Status: in-review
Stories: US2
Blocked by: 09
Assignee: coordinator (2026-11-17)
Owns: `src/features/control/command.rs`, `src/features/control/panel.rs`, `src/features/control/mod.rs`, the `Command` match in `src/interfaces/cli/repl/mod.rs`, and their tests
Traces to: D2, D8

**Why:** On 2026-11-16 the Dashboard took up to 10 s to appear, and after it was closed Gemma twice answered "it is already displayed" without calling `show_panel`. Darren said yes ("sure lets go with the rest") to fixed phrases that open it with no model call, as "close it" closes it.

**What was built:**
- `Command::Show(PanelName)`, parsed from a fixed list (`SHOW_STATS_PHRASES`): "bring up the stats", "bring up stats", "bring up the dashboard", "show the stats", "show me the stats", "show stats", "show the dashboard", "show me the dashboard", "open the stats", "open the dashboard". Case, closing punctuation and commas are ignored, and so are a leading "please", "okay", "ok" or "jarvis" and a trailing "please" or "again". So "Okay, bring up the stats again please." matches; "bring up the stats now", "don't bring up the stats" and a repeated sentence go to the model as before.
- `control::show_panel` shows the panel (again, if it is up, which restarts its timeout) and returns "Showing the stats."; a stopped panel service is a warning.
- A `COMMANDS` row, so `/help` lists it. The model path (`show_panel`) stays for every other wording.

## Acceptance criteria
- [x] The phrases above parse to `Show(Stats)` with the fillers; their nearest non-phrases stay prompts (table in `command_tests.rs`).
- [x] A show phrase shows the stats with no model request.
- [x] A stopped panel service gives a warning, not a crash.
- [x] Spot-checked: dropping " again" from the trailing fillers fails the table test.
- [x] `scripts/verify.sh` passes.
- [ ] On the Pi, "bring up the stats" by voice shows the Dashboard at once. (Pi check.)

## Comments

- 2026-11-17 (coordinator): built in the main checkout; verified with `scripts/verify.sh` together with ticket 14 (see its Comments). Not tried on the Pi.
