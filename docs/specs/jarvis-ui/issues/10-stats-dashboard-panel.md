# 10: Stats Dashboard panel

Status: done
Stories: US2, US3
Blocked by: 04, 07
Assignee: Opus agent (2026-11-16)
Owns: `ui/src/panels/`; `ui/tests/panels/`; `ui/src/link/`; `ui/tests/link/`; `ui/src/App.tsx`; `ui/src/orb/JarvisOrb.tsx`; `ui/tests/App.test.tsx`; `ui/src/preview/`; `ui/preview.html`
Traces to: D2, D7, D8, D11

**Who implements:** an Opus sub-agent (Darren, 2026-11-15). The layout is visual work that Darren judges on the Pi.

**Notes:** The last acceptance criterion needs ticket 08 (real stats) and ticket 09 (the tool) to be done. The rest can use fixtures.

**What to build:**
- The Stats Dashboard, the first Panel. It shows CPU percent, temperature, RAM used and total, and llama and BGE as up or down (glossary: Dashboard).
- It renders what arrives. It never polls.
- Showing it moves the Orb to a corner (ticket 04 takes size and position). Hiding it moves the Orb back.
- A tap outside the Panel sends one PanelClosed and hides the Panel locally. A tap on the Orb sends Tap and does not close the Panel.
- The layout fits 800x480 with no scrolling.

## Acceptance criteria
- [x] CPU, temperature, RAM, llama and BGE all render from a Stats message.
- [x] A missing temperature shows "Unavailable". It never shows 0.
- [x] Percentages round to whole numbers. 0 shows "0%", and 100 shows "100%". Tested at 0 and 100.
- [x] RAM shows used and total in GB to one decimal place. Tested at 0 and at total.
- [x] ShowPanel stats moves the Orb to its corner size. HidePanel moves it back. Tested on the props.
- [x] A tap outside the Panel sends one PanelClosed. A tap on the Orb sends one Tap and does not close the Panel.
- [x] The Panel code has no `fetch`, no polling and no `setInterval` for data.
- [x] The layout has no overflow at 800x480. Checked by eye on the Pi (ticket 05), with a screenshot in Comments.
- [x] `scripts/verify.sh` passes.

## Comments

- 2026-11-16 (coordinator): Started with an Opus agent in a worktree from `dev` at `b2d1a54`. `Owns:` widened, because the ticket as written could not be built inside it: the link must parse `show_panel`, `hide_panel` and `stats` and send `panel_closed` (`ui/src/link/`), the Orb and the Panel host must share one link (`App.tsx`, `JarvisOrb.tsx`), and the preview page must show the Dashboard from fixtures for screenshots. Decisions given to the agent, for Darren to overrule: RAM is bytes / 2^30 labelled "GB" (the 8 GB Pi then reads about 7.9, as `free -h` does); temperature in whole degrees C; before the first Stats arrives each value shows a waiting mark, never 0; a dropped connection hides the Panel; the 30 s timeout stays in Jarvis (it sends `hide_panel`), the UI has no timer; no new npm packages.
- 2026-11-16 (coordinator): Opus agent's work integrated uncommitted into `dev`; worktree removed. No npm package added.
  - **Built.** `ui/src/link/`: `protocol.ts` parses `show_panel`, `hide_panel` and `stats` as strictly as the Rust decoder and encodes `panel_closed`; `panel.ts` holds the Panel state (`applyPanelMessage`, pure); `link.ts` keeps it beside the display, notifies React subscribers only when the Panel changes, and `closePanel` sends one `panel_closed` and hides it locally. `ui/src/panels/`: `Screen.tsx` (the Orb, the tap-outside layer, the Panel frame), `Dashboard.tsx`, `format.ts`, `layout.ts`, `palette.ts` (colours from the Orb's looks), `panels.css`. `JarvisOrb.tsx` is gone: `App.tsx` owns the one link and `Screen` places the Orb.
  - **Values.** The corner Orb is 160 px, 12 px from the top-left; the Panel is 596 by 440 px on the kiosk. Figures are 68 px. The temperature meter is full at 85 C (the Pi 5's throttle point). The waiting mark is a dash.
  - **Departures from the brief, accepted.** A repeated `show_panel` while the Dashboard shows keeps the reading (no flash to the waiting marks). `"temperature_c": null` is rejected (Jarvis never sends it). No Escape key to close (no keyboard on the kiosk).
  - **Coordinator's change.** At 160 px the Orb's outer rings were cut straight by its square. The Orb now takes `round` and the corner Orb is clipped to a circle (`Orb.tsx`, `layout.ts`, one new test). The cost of the rounded clip on the Pi is not measured.
  - **Tests.** UI tests went from 102 to 220: wire boundaries (CPU -0.01, 0, 100, 100.01; RAM 0 and fractions), the Panel lifecycle, the link (one `panel_closed`, no notification for 200 levels), formatting tables (0%, 100%, 0.0 and total GB, missing temperature), layout, the Dashboard, the Screen (Orb props on show and hide, taps outside, inside and on the Orb, no React commit for levels) and the source check extended to `ui/src/panels/` (no fetch, timer or `requestAnimationFrame`). The agent broke nine boundaries by one and saw each test fail.
  - **Verified in the main checkout.** `scripts/verify.sh`: all checks passed (441 root unit tests, 220 UI tests), together with ticket 06's change; `npm run build` passes.
  - **Seen by the coordinator.** Screenshots at 800x480 from headless Chrome on the Mac in `notes/ui-spike/dashboard-shots/` (typical, no temperature, waiting, a service down, widest values, widest in Verdana, with the Orb thinking): no overflow in any.
  - **Not verified; needs the Pi and Darren.** The look on the real screen and the Pi's font; a real tap outside, inside and on the Orb; CPU and memory with the Dashboard open (D11); "bring up the stats" by voice end to end, the 30 s timeout and "close it"; real temperature, RAM and `/health` values. The Pi's UI is still the build without the Dashboard: it needs `scripts/deploy-ui.sh` (Darren's OK).
- 2026-11-16 (coordinator), later: deployed to the 8 GB Pi for Darren's check, with his OK ("sure rebuild jarvis then ill check then u may commit"). The uncommitted Rust change was built beside the checkout (`/tmp/jarvis-exp`, a detached worktree at `b2d1a54` plus the patch, with `CARGO_TARGET_DIR=~/jarvis/target`; 21.6 s) and Jarvis restarted in tmux session `jarvis` as `./target/debug/jarvis` from `~/jarvis`. `scripts/deploy-ui.sh` copied the new `ui/dist`, and Chromium was restarted with the kiosk launcher to load it. Checked: the server log shows the new bundle fetched (`index-DExZf_8s.js`), one WebSocket client is connected, `/run/user/1000/jarvis-orb.sock` no longer exists, and a `grim` screenshot shows the idle Orb in colour, full screen. The scratch worktree and files in `/tmp` are removed. Until the commit is pushed and pulled, `./run.sh` on the Pi would rebuild the old code from its checkout.
- 2026-11-16 (coordinator): Darren, after using it on the Pi: "The dashboard looks good maybe it shld include the total storage too". Jarvis's terminal there shows "bring up the stats" calling `show_panel` successfully twice. Done on his word. Gaps: no screenshot was taken on the Pi (the Mac ones are in `notes/ui-spike/dashboard-shots/`); CPU and memory with the Dashboard open were not measured; whether he tried the tap outside, the tap inside and the 30 s close one by one was not asked. Two follow-ups from the same check: "You may close the panel" reached the model, which has no tool to close a Panel (ticket 13), and he asked for storage on the Dashboard (ticket 14).
