# 04: React Orb

Status: in-review
Stories: US1
Blocked by: 02
Assignee: Opus agent (2026-11-15)
Owns: `ui/src/orb/`; `ui/src/link/`; `ui/src/preview/`; `ui/preview.html`; `ui/src/App.tsx` (the line that mounts the Orb); `ui/tests/orb/`; `ui/tests/link/`
Traces to: D2, D4, D5, D11

**Who implements:** an Opus sub-agent (Darren, 2026-11-15). It is visual work that Darren judges by eye on the Pi (ticket 05), so the coordinator relays his remarks and reviews screenshots.

**What to build:**
- The Orb as a React component. It draws the D5 look with WebGL2 (not a 2D canvas; ADR 0001) in its own `requestAnimationFrame` loop. While idle it may draw at 30 fps.
- A dev-only preview page (`ui/preview.html`, `ui/src/preview/`) renders the Orb in any state and level from URL parameters, so screenshots can be taken without Jarvis. It is not part of the production build's entry.
- The five states from visual-jarvis D2: Idle, Listening, Transcribing, Thinking and Speaking. Each has its own look. The state-to-look mapping is a pure function.
- Loudness (0 to 1, about 30 Hz) goes to the canvas loop through a ref or a small store. It never goes through React state.
- The offline look from visual-jarvis D3 (dim, grey, breathing) when there is no connection. Taps send nothing while offline.
- The `link` module connects to the Jarvis WebSocket on `127.0.0.1` at a port constant in `ui/src/link/`. That constant must match the Rust default in ticket 03, and a comment names both. The link parses ToOrb messages and sends Tap on a tap.
- Reconnect after a named delay (`RECONNECT_DELAY`). Nothing else polls.
- The Orb takes a size and a position as props, so ticket 10 can move it to a corner without changing its look.

## Acceptance criteria
- [ ] Each of the five states maps to a distinct look. The mapping is a pure function and a test checks that all five differ.
- [ ] The level clamps to 0 and 1. Tested at -0.01, 0, 1 and 1.01.
- [ ] 100 level messages cause no React re-render. The test counts renders.
- [ ] No `fetch`, no polling and no `setInterval` for data in `ui/src/orb/` or `ui/src/link/`. Only the reconnect timer exists.
- [ ] After a disconnect, the next attempt starts after exactly `RECONNECT_DELAY`. Nothing happens before it. Attempts continue. Tested with fake timers.
- [ ] Offline, the Orb draws the offline look and sends no Tap.
- [ ] A tap while connected sends exactly one Tap. A tap while connecting or offline sends nothing.
- [ ] Malformed or unknown messages are ignored and the connection stays open.
- [ ] The Orb renders at two sizes from the same state, with no change to the state mapping.
- [ ] The loop uses `requestAnimationFrame`. The 60 fps target is measured on the Pi in ticket 05. Local tests do not prove it.
- [ ] Darren judges the look on the Pi (ticket 05). No test claims that the look is right.
- [ ] `scripts/verify.sh` passes.

## Comments

- 2026-11-15 (coordinator): Darren chose a blended look (Q31): the libraries.dev "Thinking orbs" motion, animation and 3D feel plus the D1 touches. He then asked why not install the package and remodel it (Q33, "sure"): a Sonnet agent measures `thinking-orbs@0.3.2` (MIT, 2D canvas, no dependencies) on the 8 GB Pi. Approved: if it runs at about 25% of one core or less at 60 fps, install it in `ui/` at exactly 0.3.2 as the orb body, with the plexus, glyph rings and light rays as a thin WebGL2 layer. If not, the Opus agent rebuilds its motion in WebGL2. Until then the agent builds the link, state mapping, level store, renderer interface, preview page and tests, and holds the drawing.
- 2026-11-15 (coordinator): Run 3 measured `thinking-orbs@0.3.2` on the 8 GB Pi: stock component stretched from 64 px, 65 to 68% of one core; driven through `thinking-orbs/engine` on a 400 px canvas, 29 to 48%; ~490 to 525 MB PSS; ~60 fps (min 55). Over the 25% rule, but Darren accepted it ("i think its fine no? ... its only 1 core"). Installed in `ui/` at exactly 0.3.2 (licence MIT checked). D5, D11 and ADR 0001 updated; budget now about 50% of one core. Drawing path: engine on our own full-size canvas, dots tuned for 400 px, D1 touches as a WebGL2 layer, 30 fps idle cap from the animator.
- 2026-11-15 (coordinator): Opus agent built the renderer: the `thinking-orbs` engine on a centred 2D canvas (own batched painter, at most 24 fills a frame) in front of a WebGL2 layer with the D1 core, rays, HUD arcs and glyph rings (6 instanced draws, static buffers). The WebGL layer sits behind the dots, not above as briefed, because a glow on top washed the dots out; awaiting Darren's OK. Modes: offline globe (grey), idle web, listening wave, transcribing globe scan, thinking braid, speaking ring; listening and speaking follow the level. Side-by-side sheet: `notes/ui-spike/orb-shots/compare.png`. Coordinator looked at it, stubbed `getContext` in `ui/tests/App.test.tsx`, and ran `scripts/verify.sh`: all checks passed (102 UI tests). Not proven: CPU, memory and look on the Pi (ticket 05).
- 2026-11-15 (coordinator): Darren approved the glow behind the dots (Q36, "yes to all") and asked to commit and deploy. The look on the Pi is judged in ticket 05.
