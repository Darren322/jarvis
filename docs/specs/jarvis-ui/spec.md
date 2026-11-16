# Jarvis UI

Status: active
Resume here: 01 to 10 are done (Darren signed off the Orb, the kiosk and the Dashboard on the Pi on 2026-11-16; a reboot has still never been observed there). The 8 GB Pi runs the UI with the Dashboard and a Jarvis binary built from ticket 06's change before it was committed, so `./run.sh` there rebuilds old code until `dev` is pushed and pulled. In progress: 13 (a `close_panel` tool, because "You may close the panel" reached a model with no way to close one). Waiting for Darren's answers: 14 (storage on the Dashboard). Then 11 (camera, needs-info: resolution and transport, by grilling) and 12 (catalog, needs-triage).

Written 2026-11-15 from Darren's decisions. D1 was confirmed by the Pi measurement (ADR 0001). Tickets are in `issues/`; their `Status:` lines are the progress record.

## Problem Statement

The orb is a macroquad program (`crates/jarvis-orb`) that Jarvis reaches over a Unix socket. It can show only the Orb. Darren wants Jarvis to show other Panels on the same 5" touchscreen when he asks: first a Dashboard of Pi stats, then a camera view, and later Panels that Jarvis builds from the Catalog. A web page in Chromium can show all of these with less custom drawing code than a second native program.

## User Stories

1. As Darren, I want to see the Orb as before, in the new UI, so that its states and taps stay the same for me.
2. As Darren, I want to say "bring up the stats" and see the stats Dashboard, so that I can check the Pi without a keyboard.
3. As Darren, I want to close a Panel by voice ("close it") or by tapping outside it, so that the Orb comes back when I am done.
4. Later: As Darren, I want to ask for the camera and see its live image, so that I can see what the camera sees.
5. Later: As Darren, I want to ask Jarvis for a new Panel that it builds from Catalog parts, so that I can get a view I did not plan in advance.

## Solution

Chromium in kiosk mode shows a React app (`ui/`) full screen on the 800x480 touchscreen. The app draws the Orb on a 2D canvas and shows Panels. Jarvis serves the same message contract (`crates/orb-protocol`) over a WebSocket on localhost. Jarvis sends the assistant state, the loudness level and Panel messages. The app sends taps and Panel closures. Jarvis decides what each tap means. The UI holds no logic. When Jarvis is down, the page still opens from disk and shows the offline Orb.

## Implementation Decisions

| # | Decision |
| --- | --- |
| D1 | **Runtime.** A React, TypeScript and Vite app in `ui/` at the repo root. Built on the Mac with npm (lockfile committed). Deployed as a static bundle to the 8 GB Pi. Chromium shows it in kiosk mode under labwc on Wayland. No Node runs on the Pi. Confirmed by the Pi measurement on 2026-11-15; ADR `docs/adr/0001-web-ui-in-chromium.md` records the numbers and why this supersedes visual-jarvis D14. |
| D2 | **The UI holds no logic.** It renders what Jarvis sends and reports taps. Jarvis decides what a tap means (visual-jarvis D7 still holds). |
| D3 | **Contract.** `crates/orb-protocol` stays the one message contract: newline-free JSON objects. It is extended with Panel messages. The transport changes from a Unix socket to a WebSocket on localhost, served by Jarvis with `axum` (approved; exact version pinned). axum serves only the WebSocket. |
| D4 | **Offline.** A static file server separate from Jarvis serves the built UI: `python3 -m http.server --bind 127.0.0.1 --directory <repo>/ui/dist`, run as a systemd user service on the 8 GB Pi (no new dependency). Chromium opens `http://localhost:<port>`. When Jarvis is down the page still loads, shows the dim breathing offline Orb (visual-jarvis D3), and reconnects every few seconds. No tower-http. Changed 2026-11-15 from "open from disk": Chrome 157 blocks Vite's module script from `file://` (CORS, origin null) unless launched with `--allow-file-access-from-files`, which would weaken file isolation before D10's generated Panels exist. Darren chose the server ("Sure" to Q30). |
| D5 | **Orb look.** The visual-jarvis D1 look: a plexus sphere of points and links, glyph rings with invented symbols, a warm core and light rays, with no text. It is rebuilt as a React component that draws with WebGL2 in its own `requestAnimationFrame` loop (not a 2D canvas: on the Pi it took 142% of a core at 45 fps, against 25% at 60 fps for WebGL2; ADR 0001). While idle it may draw at 30 fps. Loudness updates (about 30 Hz) go straight to the canvas loop, never through React state. Changed 2026-11-15 (Darren, Q31: "blend is good ... look at the movements and the animations and 3d look of libraries.dev/orbs"): the look is a blend. Movement, animation and 3D feel follow the MIT "Thinking orbs" on libraries.dev; the D1 touches (plexus points and links, glyph rings, warm core, light rays) stay. Later on 2026-11-15 (Q34, Darren: "i think its fine"): the `thinking-orbs` package (`=0.3.2`, MIT) itself is the Orb body. Its `thinking-orbs/engine` geometry is driven on our own full-size 2D canvas (not the stock 64 px component stretched), with dots tuned for 400 px, and the D1 touches drawn as a WebGL2 layer above it. Run 3 in `notes/ui-spike/measurements-2026-11-15.md` has the numbers. |
| D6 | **Launch.** A labwc autostart entry runs Chromium in kiosk mode at login. This replaces visual-jarvis ticket 10, which is on hold. Changing the Pi needs Darren's OK. |
| D7 | **Stats Dashboard (the first Panel).** On the 8 GB Pi: CPU, temperature and RAM, from the existing `sysinfo` dependency. On the 16 GB Pi: only whether llama and BGE are up or down, from their `/health` endpoints. There is no stats service on the 16 GB Pi yet. Jarvis pushes an update once a second while the Dashboard is shown, and nothing while it is hidden. The UI never polls. |
| D8 | **Opening and closing a Panel.** A Rig tool, `show_panel`, takes a typed Panel name. The model picks the name from speech such as "bring up the stats". Jarvis may also reply aloud. The tool uses the run's one tool call. It only displays, so it needs no roadmap-8B approval step. A Panel closes on the fixed phrases "close it" or "hide that" in `control::parse` (no model call), on a tap outside the Panel, or on a timeout. The camera Panel never times out. While a Panel shows, the Orb shrinks to a corner and keeps showing state. Tapping the Orb still means listen. |
| D9 | **Camera Panel.** Pi Camera Module (not attached yet; model unknown). MJPEG from `rpicam-vid`, shown as an image. The Pi 5 has no H.264 hardware encoder. |
| D10 | **Catalog Panels (later).** Jarvis builds a Panel from the Catalog with llama.cpp `json_schema`-constrained output. Code-generated Panels from a cloud model (OpenRouter or Claude) are a later experiment. They need an explicit opt-in that covers everything sent. |
| D11 | **Resource budget.** The GUI uses about 500 MB of RAM or less, and about 50% of one core or less while the Orb idles (raised from 15% to 25% after ADR 0001's measurement, then to about 50% when Darren chose `thinking-orbs`, which measured 29 to 48% of one core at 400 px; "its only 1 core"). The Orb runs at a smooth 60 fps. A Panel may cost more while it is shown. "Not a huge impact" is acceptable to Darren. |
| D12 | **Repo.** Tests live in `ui/tests/` and use Vitest. `scripts/verify.sh` also runs the UI type-check, lint and tests. AGENTS.md gains a short "UI (TypeScript)" section. New npm packages need Darren's approval, with exact versions listed in the ticket. |
| D13 | **Order.** (1) Orb parity: states, levels, taps, offline look, kiosk launch. (2) Delete `crates/jarvis-orb`, only after Darren has seen the React Orb on the Pi. (3) Stats Dashboard. (4) Camera. (5) Catalog Panels. visual-jarvis 13's cue fix continues separately. |

## Code Facts These Tickets Build On

- `crates/orb-protocol`: the shared message types (serde only). `MAX_LINE_BYTES` is 256 today.
- `crates/jarvis-orb`: the macroquad program that is retired by ticket 06.
- `src/interfaces/orb/mod.rs`: the Unix socket server from visual-jarvis 07 and 09. Ticket 03 replaces it.
- `src/features/state.rs`: `AssistantState { Idle, Listening, Transcribing, Thinking, Speaking }`.
- `src/features/control/`: `parse` and the `COMMANDS` table.
- `src/tools/`: Rig tools and `ALLOWED_TOOLS`.
- `Cargo.toml`: `sysinfo` 0.39.6 and `serde_json` are already dependencies.

## Testing Decisions

- Follow the AGENTS Testing section. Root crate unit tests live in `tests/unit/`. Member crate tests live in `crates/<name>/tests/`. Integration tests run the binary and live in `tests/integration/`.
- Test each limit at its boundary: the last accepted value and the first rejected one. Use tables.
- UI tests use Vitest, with fake timers for the reconnect delay and the one-second Stats cadence. The UI never talks to a real network in tests.
- No real microphone, camera, screen or network. Health checks use Wiremock on localhost. Camera and stats tests use fake programs and fixed values.
- Timing uses `#[tokio::test(start_paused = true)]` on the Rust side. No wall-clock sleeps.
- The look of the Orb and the Dashboard layout are judged by eye on the Pi. Frame rate, RAM and CPU are measured on the Pi with the method written in the report.
- Local tests do not prove Pi behaviour. Each report says so.

## Out of Scope

- Code-generated Panels (D10 is a later experiment, with its own opt-in).
- Jarvis seeing through the camera (vision).
- A stats service on the 16 GB Pi.
- A phone or remote web UI.
- Any cloud request.
- Headless Jarvis, wake word and emotion colour (visual-jarvis US8 to US10).

## Approved Proposals (2026-11-15)

Darren approved all five, plus the ticket 03 dependencies.

- **P1. A `PanelClosed` message from the UI to Jarvis** (ticket 07). Without it, Jarvis does not learn about a tap outside the Panel, so it keeps pushing Stats.
- **P2. A Panel timeout of 30 seconds** for the Stats Dashboard (ticket 09), set in config. The camera never times out (D8).
- **P3. Tickets 08 and 09 are also blocked by 03.** Both publish through the transport that 03 builds, so they need its handle.
- **P4. Camera frames and D3.** Ticket 11 decides, once the camera is attached; the likely answer is one `/camera` route on the same axum server.
- **P5. The Catalog part limit** (ticket 12): at most 8 parts per Panel, as a starting point to revisit in ticket 12.
- **Ticket 03 dependencies.** axum's `ws` feature, and `tokio-tungstenite` as a dev-dependency WebSocket test client. Exact versions are pinned in ticket 03.

## Status

| Ticket | Title | Status | Blocked by |
| --- | --- | --- | --- |
| 01 | Record the UI runtime decision | done | None |
| 02 | UI scaffold | done | None |
| 03 | WebSocket transport | done | None |
| 04 | React Orb | done | 02 |
| 05 | Kiosk launch and deploy | done | 03, 04 |
| 06 | Retire jarvis-orb | done | 05 |
| 07 | Panel messages | done | None |
| 08 | Stats source | done | 03, 07 |
| 09 | show_panel tool and close phrases | done | 03, 07 |
| 10 | Stats Dashboard panel | done | 04, 07 |
| 11 | Camera panel | needs-info | 10 |
| 12 | Catalog panels | needs-triage | 11 |
| 13 | Close a Panel by saying it any way | in-progress | 09 |
| 14 | Storage on the stats Dashboard | needs-info | 10 |
