# 12: Catalog panels

Status: in-review
Stories: US5
Blocked by: 11
Assignee: Opus agent, high effort (2026-11-21)
Owns: `crates/orb-protocol/src/lib.rs` and its tests (the built Panel's messages and parts); `src/tools/build_panel.rs` (new) and `src/tools/mod.rs` (register it and add it to `ALLOWED_TOOLS`); `src/features/panel/mod.rs` (showing a built Panel, its timeout arm); `src/features/stats/mod.rs` (stats flow while a built Panel shows); `src/features/camera/mod.rs` (only the `is_camera` arm for the new name); `src/features/control/` only if a match on `PanelName` needs an arm; `src/app.rs` (wiring only); `ui/src/panels/`, `ui/src/link/` and `ui/tests/`; the matching tests under `tests/unit/`. `Cargo.toml`, `Cargo.lock` and `ui/package*.json` are the coordinator's: no new crate or npm package.
Traces to: D2, D3, D7, D8, D10, P5

**Settled with Darren on 2026-11-21 (grilling, "yes to all"):**

Darren's examples, which every decision is checked against:

| # | Darren would say | What it needs | In this ticket |
| --- | --- | --- | --- |
| E1 | "show my facts as a list" | A fixed list; the facts are already in the prompt | Yes |
| E2 | "put a 10-minute timer on the screen" | A live countdown and something when it ends | No: ticket 17 |
| E3 | "show me the CPU and temperature as gauges" | Live gauges fed by Jarvis's stats, as the Dashboard is | Yes |
| E4 | "show today's calendar" | A calendar tool (roadmap step 5) | No: after the calendar |
| E5 | "make me a shopping list" | Showing it is a fixed list; keeping and adding to it needs storage | Showing only |

| # | Decision |
| --- | --- |
| G1 | **A `build_panel` tool** (Q2). Its arguments are the Panel; llama.cpp constrains them to the tool's schema, as for `show_panel`. It is the run's one tool call; the policy is unchanged. It only displays, so no roadmap-8B approval. This is how D10's "json_schema-constrained output" is met. |
| G2 | **Fixed and live content** (Q3). Card and list text is written by the model once. A gauge names a live source and Jarvis fills its value; the model never writes a number. |
| G3 | **The Catalog in v1** (Q4): `card` (title, text), `list` (title, items), `gauge` (source). No chart (no history kept yet), no camera tile (the camera Panel does that), no timer (ticket 17). Update the glossary's Catalog entry when this ships. |
| G4 | **Limits** (Q5): a Panel title of at most 40 characters; 1 to 4 parts; a list has 1 to 8 items; a card's title and text, a list's title and each item at most 120 characters (Unicode scalar values, `chars()`). Each is a named constant, tested at its edge (P5's 8 parts is replaced by 4). |
| G5 | **Gauge sources** (Q6): exactly the Dashboard's figures: `cpu`, `temperature`, `ram`, `storage`, `llama`, `bge`. While a built Panel shows, Jarvis sends the same `Stats` messages it sends for the Dashboard, once a second, and the UI's gauges read from them, so a gauge and the Dashboard never disagree. A missing temperature or storage shows as unknown, never 0 (as the Dashboard does). |
| G6 | **Timeout** (Q7): like the stats Panel, the configured `dashboard_timeout` (30 s by default); "close it", "hide that", `close_panel` and a tap outside close it as any Panel. |
| G7 | **Rejected output** (Q8): Jarvis checks every limit before showing anything. A rejected Panel shows nothing, and the tool returns a typed error whose message says what was wrong ("a panel has at most 4 parts"), so Gemma tells Darren it could not build it. No terminal notice. |
| G8 | **The UI holds no logic** (D2): it lays out the parts it is sent and reads gauge values from `Stats`; it never runs anything from the model. Text from the model is shown as text, never as HTML. |

**What to build:**
- **Protocol:** a `Built` variant of `PanelName` (wire `"built"`) for the Panel state, and one new `ToOrb` message that carries the built Panel (title and parts) and opens it, so the UI gets content and "show" in one message. `HidePanel` closes it like any Panel. Parts are a tagged enum (`{"kind":"card",...}`, `{"kind":"list",...}`, `{"kind":"gauge","source":"cpu"}`), with `deny_unknown_fields`.
- **Tool:** `build_panel` with typed `Args` (`deny_unknown_fields`), a schema that lists the three parts and their limits, a description that names the examples (E1, E3, E5) and says a gauge's value comes from Jarvis, and a typed error enum for the limits. The tool asks the Panel state to show the built Panel.
- **Panel state:** showing a built Panel replaces whatever shows, with the stats timeout; the stats source runs while it shows.
- **UI:** a built-Panel component in the Panel frame (the same frame, colours and Orb corner as the Dashboard), laying out 1 to 4 parts; gauges reuse the Dashboard's look and its formatting of each figure.

## Acceptance criteria
- [x] The built Panel message and each part round-trip on the wire; unknown kinds, unknown sources and unknown fields are rejected (protocol tests, Rust and TypeScript).
- [x] Limits at their edges (one table): 0 parts rejected, 1 accepted, 4 accepted, 5 rejected; 0 list items rejected, 8 accepted, 9 rejected; 120 characters accepted and 121 rejected for a card title, card text, list title and list item (one multi-byte case); a Panel title of 40 accepted, 41 rejected. Each boundary test spot-checked by breaking it by one.
- [x] A rejected Panel sends nothing to the orb and its error message names the limit; a part not in the Catalog is rejected by the argument rule.
- [x] Streamed model test (`MockCompletionModel::from_stream_turns`): a `build_panel` call shows the Panel, and the second turn answers.
- [x] The built Panel times out with the stats timeout (paused time) and closes on "close it" and on `PanelClosed`; showing the stats or the camera replaces it.
- [x] `Stats` messages flow while a built Panel shows and stop when it closes.
- [x] UI (Vitest): each part renders; a gauge shows the value from the latest `Stats` and "unknown" for a missing temperature or storage; model text is rendered as text (a `<b>` in a card stays literal); the Orb stays in its corner; a tap outside sends `PanelClosed`.
- [ ] The token cost is stated: the tool definition's size in tokens (counted with the Gemma server's read-only `/tokenize` on the Pi, by the coordinator) and the longest Panel the schema allows, both against the 4,096-token slot.
- [x] `scripts/verify.sh` passes; any `select!`, cancellation or timing test also passes `scripts/verify.sh --repeat <pattern>` (20 runs).
- [ ] Pi (coordinator, with Darren): E1, E3 and E5 by voice; what Gemma builds and how it looks.
- [x] No cloud request (D10's code-generated Panels are a separate opt-in experiment).

## Comments

- 2026-11-21 (coordinator): grilled with Darren after ticket 11 closed. Q1: "sure" to the five examples above, and for the future "i will ask jarvis modify itself with external harness or openrouter api call via our own rust subagents/agents" (recorded as roadmap step 12, not part of this ticket). Q2 "sure" (a tool). Q3 "I think both is good". Q4 to Q8: "yes to all". The timer (E2) is ticket 17.
- 2026-11-21 (coordinator): built by an Opus agent (high effort) in the main checkout. Reviewed the diff against G1 to G8 and ran `scripts/verify.sh` in the main checkout: all checks passed (the agent's `--repeat buil` was 20 of 20). The limits live in `orb_protocol` (`MAX_PANEL_TITLE_CHARS` 40, `MAX_PANEL_PARTS` 4, `MAX_LIST_ITEMS` 8, `MAX_PART_TEXT_CHARS` 120; `BuiltPanel` is valid by construction) and the tool's schema is built from them. `ToOrb::ShowBuiltPanel` carries the Panel (`{"type":"show_built_panel","title":...,"parts":[{"kind":"list",...},{"kind":"card",...},{"kind":"gauge","source":"cpu"}]}`); `PanelName::Built` cannot be named by a message or `show_panel`. Over-limit Panels reach the tool and return a typed error whose message the model sees (`map_error` to `invalid_args`); unknown kinds, sources or fields are stopped by the argument rule. Gauges read the Dashboard's `Stats` and draw with the Dashboard's own `Gauge`. A source scan bans `innerHTML` and `dangerouslySetInnerHTML`. `ToOrb` is no longer `Copy`; the `ToOrb` line limit is `MAX_TO_ORB_LINE_BYTES` (26,452, the widest escaped Panel); `FromOrb` keeps 274. The glossary's Catalog entry is updated (G3).
    - **Risk for Darren (estimate):** every request caps the answer at 1,024 tokens, and the largest Panel the schema allows is about 4,360 characters of text, likely more than 1,024 tokens. Gemma could start such a Panel and not finish it; the policy stops a truncated turn, so nothing shows. It fails safe; E1 with many long facts could hit it.
