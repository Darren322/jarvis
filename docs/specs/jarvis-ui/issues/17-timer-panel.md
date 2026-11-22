# 17: A timer on the screen

Status: wontfix
Stories: US5
Blocked by: 12
Assignee: unassigned
Owns: undecided
Traces to: D2, D8, ticket 12 (E2)

**Why:** Darren's example E2 in ticket 12: "put a 10-minute timer on the screen". A countdown is a display, but a timer must also do something when it ends, which a Catalog part does not. Split from ticket 12 on 2026-11-21 (Q4).

**Open questions for Darren (by grilling):**
- What happens at zero: a sound cue, Jarvis saying "Time's up" (the REPL's announcements from ticket 11 can carry it), both, or the Panel flashing?
- Does the timer keep running if the Panel is closed or times out, and can it be asked about ("how long is left")?
- One timer at a time, or several? Cancelling by voice ("cancel the timer")?
- Does it survive a Jarvis restart (needs storage) or not?
- Where the countdown is computed: Jarvis sends the end time and the UI only draws the remaining time (D2: no logic in the UI).

## Acceptance criteria
- To be written when the questions are settled.

## Comments

- 2026-11-22 (coordinator): moved to its own spec, [timers](../../timers/spec.md). The grilling settled every open question above, and the answers made this a feature, not a UI ticket: a model run when a timer ends, storage at schema v7, voice commands, and Darren's confirmation before each timer is set (8B). Closed here as `wontfix` because the work continues in that spec.
