# 10: Stats Dashboard panel

Status: ready-for-human
Stories: US2, US3
Blocked by: 04, 07
Assignee: unassigned
Owns: `ui/src/panels/`; `ui/tests/panels/`; the one line in `ui/src/App.tsx` that mounts the Panel host
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
- [ ] CPU, temperature, RAM, llama and BGE all render from a Stats message.
- [ ] A missing temperature shows "Unavailable". It never shows 0.
- [ ] Percentages round to whole numbers. 0 shows "0%", and 100 shows "100%". Tested at 0 and 100.
- [ ] RAM shows used and total in GB to one decimal place. Tested at 0 and at total.
- [ ] ShowPanel stats moves the Orb to its corner size. HidePanel moves it back. Tested on the props.
- [ ] A tap outside the Panel sends one PanelClosed. A tap on the Orb sends one Tap and does not close the Panel.
- [ ] The Panel code has no `fetch`, no polling and no `setInterval` for data.
- [ ] The layout has no overflow at 800x480. Checked by eye on the Pi (ticket 05), with a screenshot in Comments.
- [ ] `scripts/verify.sh` passes.

## Comments
