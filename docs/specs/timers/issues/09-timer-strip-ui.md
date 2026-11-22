# 09: The timer strip on the screen

Status: ready-for-agent
Stories: US4, US5
Blocked by: 05
Assignee: unassigned
Owns: `ui/src/panels/Timers.tsx` (new), `ui/src/panels/timers.css` (new), `ui/src/panels/**` (only what the strip needs), `ui/src/link/link.ts`, `ui/src/link/display.ts`, `ui/src/App.tsx`, `ui/src/preview/**`, `ui/tests/**` (strip tests). Not `ui/src/link/protocol.ts` (ticket 05). `ui/package*.json` are the coordinator's: no new npm package.
Traces to: D6, D11, D14, D15

**Opus agent, with the UI skills (frontend-ui-engineering, vercel-react-best-practices, react-testing, web-design-guidelines).** Name them in the brief. Darren judges the look on the Pi.

**What to build:** The strip of D15 at the top right.
- Shows every timer in the last `timers` message: label (or "Timer"), countdown from `ends_at_ms` and the UI's own clock, one row per timer, at most 5. A `pending` row shows **Set** and **Don't**; a `running` row shows the countdown; an `ended` row flashes and shows "Ended" until the next message drops it (Jarvis sends the 2-minute removal, ticket 03).
- It is there whether or not a Panel is open, and stays clear of the Orb when the Orb is in its corner (check the Orb corner layout in `ui/src/panels/layout.ts`).
- **Taps:** Set sends `confirm_timer`, Don't sends `reject_timer`. Tapping a running row asks "Cancel 'Time to sleep'?" with **Cancel** and **Keep**; Cancel sends `cancel_timer`, Keep closes the question. Tapping an ended row sends `cancel_timer` with its id (D17), with no question first. The UI holds no logic: it decides nothing, it renders what it is sent and sends ids back (jarvis-ui D2). Label text is shown as text, never as HTML.
- Countdown format: "m:ss" under an hour, "h:mm:ss" after; it never shows a negative time (a row at zero shows "0:00" until the ended message arrives). The ticking uses one timer for the strip, not one per row, and is cleaned up on unmount.
- Fixtures for the preview page (`ui/src/preview/fixtures.ts`) for 0, 1, 5 timers, a pending, a running and an ended one.

## Acceptance criteria
- [ ] Component tests (Vitest, React Testing Library) cover: 0 rows (nothing drawn), 1 and 5 rows; each state's controls; the countdown at 59:59, 1:00:00 and 0:00 (fake timers); a message with 5 timers renders all 5.
- [ ] Taps send the right message with the right id: Set, Don't, Cancel after the question, and nothing for Keep.
- [ ] A long label (120 characters) does not break the layout (truncated with an accessible full text) and markup in a label is shown as text.
- [ ] The strip does not overlap the Orb in its corner or an open Panel's frame (verified in the preview page at the Pi's screen size, recorded in the Comment with how it was checked).
- [ ] The strip is keyboard and screen-reader sensible: buttons are buttons with names, the flash respects `prefers-reduced-motion`.
- [ ] `npm run build`, `typecheck`, `lint` and `npm test` pass in `ui/`; `scripts/verify.sh` passes. No new npm package.
- [ ] Pi check (Darren's eyes): T1's pending timer shows Set and Don't, a running timer counts down, an ended one flashes, and the strip stays clear of the Orb and of the Panels. He judges the look.

## Comments
