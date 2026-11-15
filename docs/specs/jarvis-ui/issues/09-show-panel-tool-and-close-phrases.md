# 09: show_panel tool and close phrases

Status: in-progress
Stories: US2, US3
Blocked by: 03, 07
Assignee: Sonnet agent (2026-11-15)
Owns: `src/tools/show_panel.rs` (new); `src/tools/mod.rs` (the `show_panel` registration and its `ALLOWED_TOOLS` entry); `src/features/panel/` (new: the panel state and its timer); `src/features/control/command.rs` and the other `src/features/control/` files that the close command needs; `tests/unit/tools/show_panel_tests.rs`; `tests/unit/features/panel/`; `tests/unit/features/control/`
Traces to: D2, D8

**Notes for the coordinator:**
- Blocked by 03 as well as 07 (proposal P3). The panel state publishes through the transport that 03 builds.
- The `pub(crate) mod panel;` line in `src/features/mod.rs` and the wiring in `src/app.rs` are done by the coordinator at integration. Ticket 08 also needs one line in `src/features/mod.rs`.
- Timeout `PANEL_TIMEOUT` of 30 seconds for the Dashboard, set in config (proposal P2, approved 2026-11-15).

**What to build:**
- A Rig tool `show_panel` with a typed `panel` argument. Its only accepted value today is `stats`. `#[serde(deny_unknown_fields)]`. It is added to `tools::register` and to `ALLOWED_TOOLS`. It needs no approval step: it only displays (D8).
- The tool tells the panel state to show that Panel. The panel state sends ShowPanel through the handle from ticket 03. Showing a Panel replaces the one already shown. Only one Panel shows at a time.
- The close phrases "close it" and "hide that" are matched in `control::parse`, after trimming and ignoring case. They produce a Close command. No model call is made.
- A PanelClosed message (proposal P1) closes the Panel. A Tap on the Orb does not close it. It still means listen (D8).
- The Panel times out after `PANEL_TIMEOUT` and sends HidePanel. The timeout rule is a function of the Panel kind. The camera arm is added in ticket 11.
- The tool result tells the model that the Panel is shown. Jarvis may reply aloud. The run's one tool call is this call (AGENTS policy).

## Acceptance criteria
- [ ] `ALLOWED_TOOLS` lists `show_panel`. `registered_tools_match_the_allowed_list` passes.
- [ ] A `panel` of `stats` is accepted. Unknown names such as `weather` and `Stats` are rejected. An extra field is rejected.
- [ ] Close phrases table: "close it", "Close it.", "hide that" and "  HIDE THAT  " produce Close. "close" alone, "close the stats", and "don't close it yet" do not.
- [ ] A close phrase makes no model call. Tested with a mock that fails if called.
- [ ] Timeout: at `PANEL_TIMEOUT` minus one millisecond the Panel is still shown. At `PANEL_TIMEOUT` it is hidden and HidePanel is sent. Paused time.
- [ ] A PanelClosed message hides the Panel. A Tap while a Panel shows produces listen and leaves the Panel shown.
- [ ] A second Panel replaces the first. Only one ShowPanel is pending at a time.
- [ ] The existing policy hook accepts `show_panel` as the run's one tool call. A second tool call is still rejected (test).
- [ ] `scripts/verify.sh` passes. Each new boundary test was seen to fail when its limit moved by one.
- [ ] Local tests only. The report says so.

## Comments
