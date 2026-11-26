# Timers

Status: closed
Resume here: closed on 2026-11-26. All 9 tickets are done, and the feature is on the 8 GB Pi through `baa6b6c`; Darren confirmed it there ("the feat works"). What it took on the Pi is in ticket 04's 2026-11-26 Comments. Leftovers: the timer-ended WAV (D21, silent until Darren picks one); the model's printed text after a proposal still says "I have set a timer" (not spoken, but kept in history); after the USB speaker drops, the cues stay silent until the next answer, and cpal 0.17.3's stuck worker thread spins until a restart (a cpal upgrade needs approval); three wrong noted facts (ids 11 to 13) wait for Darren; and `a_websocket_client_that_leaves_closes_the_panel` failed once under full load. Google Calendar is the next spec.

Written 2026-11-22 from Darren's decisions (grilling on jarvis-ui ticket 17, rounds Q1 to Q20; "Sure no problem"). Replaces [jarvis-ui 17](../jarvis-ui/issues/17-timer-panel.md). Tickets are in `issues/`; their `Status:` lines are the progress record.

## Problem Statement

Jarvis cannot keep time for Darren. He wants to say "set a timer for 11 pm, time to sleep" and have Jarvis remind him then, in words that fit the moment, not a bare beep. A countdown is something on the screen, but a timer also has to act when it ends, wake Jarvis and survive a restart. No Panel or Catalog part does that (jarvis-ui 12, Q4).

## Darren's example sentences

Every decision is checked against these before a ticket starts.

| # | Darren's sentence | What it does |
| --- | --- | --- |
| E2 | "put a 10-minute timer on the screen" | Proposes an unlabelled 10-minute timer; it is set once Darren confirms |
| T1 | "Please help me set a timer for 11 pm, time to sleep" | Proposes "Time to sleep" at 11:00 pm (tomorrow if 11 pm has passed); set once confirmed |
| T2 | "hey jarvis, could you set up a timer for me in 25 minutes to do work" | Proposes "Time to do work" in 25 minutes; set once confirmed |
| T3 | "jarvis, set a timer for 10 minutes" | As E2 |
| T4 | "cancel the timer" / "cancel the sleep one" | Cancels the only timer, or the one whose label matches |
| T5 | "how long is left" | Says what is left on the running timers |
| T6 | "yes" / "no" (after Jarvis asks) | Sets or drops the pending timer |

## User Stories

1. As Darren, I want to set a timer by voice, for a duration or a clock time and with an optional label, so that Jarvis reminds me later.
2. As Darren, I want Jarvis to ask me before it sets a timer, and to set it only when I say yes or tap Set, so that a misheard sentence never sets one.
3. As Darren, I want Jarvis to react when a timer ends, with a sound and a spoken line that fits the label, so that "Time to sleep" sounds like a reminder and not a beep.
4. As Darren, I want to see every pending, running and ended timer at the top right of the screen with its countdown, so that I know what is set.
5. As Darren, I want to cancel a timer by voice or by tapping it, so that I can change my mind.
6. As Darren, I want to ask how long is left, so that I don't have to look at the screen.
7. As Darren, I want timers to survive a Jarvis restart, so that a restart or a crash doesn't lose them.

## Implementation Decisions

Settled with Darren on 2026-11-22.

| # | Decision |
| --- | --- |
| D1 | **One-off timers only, for now.** Repeating timers ("every night at 11 pm") and snooze ("+5 minutes" on an ended timer) are later tickets in this spec, once one-off timers work on the Pi. |
| D2 | **Duration or clock time.** "In 25 minutes" or "at 11 pm". A clock time that has already passed today means tomorrow. The time zone and clock come from `jiff` (already used by `current_time`). Jarvis says the resolved time back ("11:00 pm tomorrow"). |
| D3 | **At most 5 timers** (`MAX_TIMERS`), pending ones included. A 6th is refused and Jarvis says why. |
| D4 | **Darren confirms every new timer** (roadmap 8B's approval, tied to this one timer). `set_timer` only creates a *pending* timer. The strip shows it with **Set** and **Don't**, and Jarvis says a fixed line: "Set 'Time to sleep' for 11:00 pm tomorrow? Say yes or tap Set." Saying "yes", or tapping Set, within 30 seconds (`TIMER_CONFIRM_WINDOW`) sets it. "No", tapping Don't, or the window running out drops it. `control::parse` understands "yes" and "no" only while a timer is pending; the model never confirms anything. Anything else said while a timer is pending goes to the model as usual, and the window keeps running. |
| D5 | **Access level on `ToolRule`.** This is the first tool that writes or acts (the AGENTS.md trigger), so `ToolRule` gains an access level. `set_timer` is marked as needing approval; `cancel_timer` and `list_timers` are not. The approval prompt belongs to the interface, not the tool. |
| D6 | **At zero:** a new sound cue plays at once (a WAV in `data/sounds/`, loaded like the state sounds) and the row flashes. Once Jarvis is idle, it starts a model run on its own. The run gets a fixed line from Jarvis ("The timer 'Time to sleep', set at 10:30 pm, has ended.") plus the normal history and facts list. It may call **no tools** (as D20), so a label cannot make Jarvis act. Its answer is spoken, and archived and added to the history like any other turn. The ended row stays until tapped away or 2 minutes pass (`ENDED_TIMER_SHOWN_FOR`). |
| D7 | **Busy at zero:** if Jarvis is recording, thinking or speaking, the model run waits until idle, like the announcements of jarvis-ui 11 (C6, read only in the REPL's idle `select!`). The strip shows the timer as ended right away. |
| D8 | **The 30-minute rule applies:** a timer run is a prompt like any other for fast-memory 09. After 30 minutes of quiet it starts a fresh conversation first, and its answer counts as the last answer. |
| D9 | **Timers that ended while Jarvis was off:** at start, one less than 1 hour overdue (`MISSED_TIMER_GRACE`) runs late and says so ("ended 40 minutes ago while I was off"). One more overdue is dropped, with a printed notice. |
| D10 | **Cancelling by voice needs no confirmation.** Jarvis says "Cancelled 'Time to sleep'." With no label and exactly one timer, cancel that one. With a label, cancel the single timer whose label contains the words. With zero or several matches, the tool returns the running list and Jarvis asks which. |
| D11 | **Cancelling on the screen:** tapping a row asks "Cancel 'Time to sleep'?" with Cancel and Keep. Confirming cancels it, and Jarvis says a fixed line; there is no model run. |
| D12 | **Three tools in `src/tools/timers/`:** `set_timer` (`label` optional; `minutes` or `at`), `cancel_timer` (`label` optional) and `list_timers`. They are separate tools because only `set_timer` needs approval, and the arguments stay flat because Gemma's chat template cannot show nested `anyOf` (jarvis-ui 12). That makes 8 tools, which hits the grouping trigger, but **every tool is still offered on every request**: changing the tool list changes the start of the prompt and empties Gemma's cache (fast-memory 10: 9.6 s vs 0.8 s). The AGENTS.md trigger table records why. |
| D13 | **Storage:** its own `storage/timers/` module with its own types (a `TimerId` newtype, label, end time, state `pending` / `running` / `ended` / `cancelled`), at schema v7. It comes after a refactor ticket that moves each migration's SQL into `storage/migrations/vN.sql` (the "about six versions" trigger), with no change in behaviour. Darren backs up the database before v7 runs on the Pi. |
| D14 | **Screen contract**, in `crates/orb-protocol` (jarvis-ui D3). Jarvis sends `{"type":"timers","timers":[{"id":3,"label":"Time to sleep","ends_at_ms":…,"state":"running"}]}` on every change and whenever the UI connects. The UI sends `{"type":"cancel_timer","id":3}`, `{"type":"confirm_timer","id":3}` and `{"type":"reject_timer","id":3}`. Jarvis checks each id against storage; the UI decides nothing (jarvis-ui D2). |
| D15 | **The strip** sits at the top right whether or not a Panel is open, and stays clear of the Orb when the Orb is in its corner. The UI works out the remaining time from `ends_at_ms` and its own clock (same Pi, same clock). Darren judges the look on the Pi; an Opus agent builds it with the UI skills. |
| D16 | **After a run that proposed a timer, Jarvis speaks only the fixed line of D4.** The model's own reply is still printed and kept in the history unchanged; the fixed line is the one place the wording must be exact, because it is what Darren says "yes" to. (Settled 2026-11-22, Q21.) |
| D17 | **Tapping an ended row** sends `cancel_timer` with its id; Jarvis removes the row and says nothing. (Q22.) |
| D18 | **`set_timer`'s `at` is 24-hour `"HH:MM"`.** The model turns "11 pm" into `"23:00"`; Jarvis's own code decides today or tomorrow (D2). (Q23.) |
| D19 | **Jarvis removes an ended row after 2 minutes** and sends the new list; the UI only draws what it is sent. (Q24.) |
| D20 | **"No tools" in D6 means the tool list stays in the request with tool choice `none`**, as on the second turn, so Gemma's prompt cache stays warm (fast-memory 10); a tool call in the answer is rejected and runs nothing. Not a request with no tools listed: that would start this run, and the next question, from a cold cache. (Q25, "lets go with option 1".) |
| D21 | **Darren picks the timer-ended sound**, a WAV next to the state sounds in `data/sounds/` on the Pi (not in git). Until it is there the cue is a warning and silence. (Q26.) |
| D22 | **A timer still pending when Jarvis restarts is deleted at start**, with no spoken line; its 30-second window is over. (Q27.) |
| D23 | **A new proposal replaces a timer still pending**, without a word: at most one timer waits for a yes, so "yes" always means the one Jarvis asked about last. (Settled 2026-11-22, Q28: "yes okay".) Longest timer: 24 hours (`MAX_TIMER_MINUTES` = 1,440), accepted the same day. |

## Testing Decisions

- Resolving "in 25 minutes" and "at 11 pm" into an end time is a pure function of a `jiff::Zoned` and the request, tested with fixed instants and zones, including a clock time just before and just after the present.
- Limits are tested at their boundaries: 5 vs 6 timers, the confirm window at 30 s vs just past it, missed timers at just under vs just over 1 hour, the ended row at 2 minutes.
- The timer service is tested with `#[tokio::test(start_paused = true)]`; nothing waits on the wall clock. Races (a timer ending during a recording, confirm against expiry) pass `scripts/verify.sh --repeat` 20 times.
- The v7 migration test builds a v6 database with realistic rows (AGENTS.md "Changing the schema"). The migration-file refactor shows the same `cargo test -- --list` inventory and identical schema results before and after.
- The model run at zero is scripted with `MockCompletionModel::from_stream_turns`, and a test checks that it lists every tool with tool choice `none` (D6 read with D12: the cache-safe way to forbid a call) and that a tool call in its answer runs nothing.
- Only the Pi can show that Gemma picks the right tool and arguments for Darren's sentences, and that the strip looks right.

## Out of Scope

- Google Calendar. It is roadmap step 5 (8A read, 8B write) and becomes its own spec after this one. A Google API key reads only public calendars, so Darren's calendar needs an OAuth login; and as Jarvis's first cloud request, it needs his explicit opt-in to what is sent.
- Repeating timers and snooze (D1: later tickets here).
- Alarms that wake the screen or the Pi from sleep.
- Timers for other people (roadmap 12D).

## Further Notes

Likely tickets, for `/to-tickets` to confirm:

1. Refactor: the migration SQL moves into `storage/migrations/vN.sql`, with no change in behaviour.
2. Storage: the timers table at v7, its types and SQL.
3. The timer service: tracking the next end, firing, the rule for timers missed while off, pending expiry. Concurrency, so the coordinator implements it.
4. The three tools, and the access level on `ToolRule`.
5. Confirming: "yes" and "no" in `control::parse` while a timer is pending, and the fixed lines.
6. The model run at zero (no tools, waits for idle, the 30-minute rule) and the new sound cue.
7. The screen messages in `orb-protocol` and the Rust side of the interface.
8. The UI strip (an Opus agent).
