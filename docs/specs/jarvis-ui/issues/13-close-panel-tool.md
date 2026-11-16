# 13: Close a Panel by saying it any way

Status: in-progress
Stories: US3
Blocked by: 09
Assignee: coordinator (2026-11-16)
Owns: `src/tools/close_panel.rs` (new); `src/tools/mod.rs`; `tests/unit/tools/`; the test agents under `tests/unit/` that register the tools by hand
Traces to: D2, D8

**Why:** On 2026-11-16 Darren said "You may close the panel" with the Dashboard open, and Jarvis answered "I am not authorized to close the panel, master." Only the exact phrases "close it" and "hide that" close a Panel without the model (D8). Everything else goes to Gemma, which has `show_panel` but no way to close one, so it followed its system prompt and explained the limitation. Darren: "cant the local llm deduce it or smth?" It did understand; it had no tool.

**What to build:**
- A Rig tool, `close_panel`, with no arguments. It closes the Panel that is shown and tells the model so; with no Panel shown it says that and changes nothing. Like `show_panel` it only changes the display, so it needs no approval step (D8).
- It is registered in `tools::register` and listed in `ALLOWED_TOOLS`.
- The fixed phrases "close it" and "hide that" stay: they close at once, with no model call. The tool is the slower path (two model turns) for every other wording.
- The test agents build their tools with `tools::register` instead of listing each tool by hand, so the next tool does not need 28 edits.

**Decided by the coordinator, for Darren to overrule:** the fixed phrase list is not widened in this ticket. If closing through the model feels too slow on the Pi, more phrases can be added to `is_close_phrase`.

## Acceptance criteria
- [ ] `close_panel` accepts `{}` and rejects any argument; the same rule holds on the allowed list.
- [ ] Calling it with the stats Panel shown sends one `HidePanel` and tells the model the Panel is closed.
- [ ] Calling it with no Panel shown sends nothing and tells the model none was open.
- [ ] A streamed run through the real registration and policy hook accepts `close_panel` as the run's one tool call.
- [ ] `registered_tools_match_the_allowed_list` passes with three tools.
- [ ] `scripts/verify.sh` passes.
- [ ] On the Pi, Darren closes the Dashboard with a sentence that is not a fixed phrase. (Pi check.)

## Comments
