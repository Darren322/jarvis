# Jarvis knows the time

Status: active
Resume here: 01 (the `current_time` tool) is in review since 2026-11-21: integrated uncommitted in `dev`, `scripts/verify.sh` green; next, deploy and Darren's voice check. Darren approved the design and the `jiff` dependency on 2026-11-21.

Written 2026-11-21 from Darren's decisions. Tickets are in `issues/`; their `Status:` lines are the progress record.

## Problem Statement

Jarvis cannot tell the time. Nothing in the system prompt gives the date or time and no tool returns it, so "what time is it?" or "what day is it?" gets an invented answer.

## User Stories

1. As Darren, I want to ask Jarvis the time, the date or the day of the week and hear the Pi's real local time, so that I can trust the answer.

## Implementation Decisions

Settled with Darren on 2026-11-21 ("okay sure ig").

| # | Decision |
| --- | --- |
| D1 | **A tool, not a note in every request.** A `current_time` tool beside `system_status`. Only time questions pay for it (a second model turn); every other request is unchanged. Not a note in the system prompt: a preamble that changes every minute empties the server's prompt cache (fast-memory 10). |
| D2 | **`jiff` reads the clock and the time zone** (`jiff = "=0.2.37"`, default features, approved by Darren). On the Pi it reads the system zone from `/etc/localtime` and the zone data from `/usr/share/zoneinfo`; nothing is bundled. std cannot give local time. |
| D3 | **Output:** one small JSON object with the local time, the weekday, the date and the time zone, shaped like `system_status`'s output. The model turns it into speech. |

## Testing Decisions

- The formatting is a pure function of a `jiff::Zoned`, tested with fixed instants and zones; no test reads the real clock.
- The tool's registration is covered by the existing `registered_tools_match_the_allowed_list`.
- Only the Pi can show that Gemma calls the tool for spoken time questions.

## Out of Scope

Timers, alarms, reminders, other cities' time, and the calendar (roadmap 8A).
