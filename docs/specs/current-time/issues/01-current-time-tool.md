# 01: A `current_time` tool

Status: in-review
Stories: US1
Blocked by: None
Assignee: Sonnet agent (2026-11-21)
Owns: `src/tools/current_time.rs` (new), `src/tools/mod.rs` (only the `pub mod`, `use`, `.tool(...)` and `ALLOWED_TOOLS` lines), `tests/unit/tools/current_time_tests.rs` (new); and, only if their expected tool lists must change to compile or pass, `tests/unit/clients/local_llm_tests.rs` and `tests/unit/features/assistant/*_tests.rs`
Traces to: D1, D2, D3

**What to build:**
- A Rig tool named `current_time`, built the way `system_status` is (`src/tools/system_status_tool.rs`): empty typed `Args` with `#[serde(deny_unknown_fields)]`, `Error = Infallible`, a description that tells the model to use it for any question about the current time, date or day of the week, and never to guess them.
- It reads `jiff::Zoned::now()` and returns a small serializable struct. Proposed fields (example for the Pi at 14:30 on 2026-11-21): `{"time":"14:30","weekday":"Saturday","date":"2026-11-21","time_zone":"Asia/Singapore","utc_offset":"+08:00"}`. 24-hour time, no seconds; an ISO date; the weekday in English.
- The formatting is a pure function from `&jiff::Zoned` to that struct, so tests pass fixed instants. No clock trait.
- When the system zone cannot be read, jiff falls back to `Etc/Unknown` (UTC). The tool reports the zone as it is (`time_zone: "Etc/Unknown"`) and the description tells the model to say the time zone is unknown in that case, not to guess one.
- Register it in `tools::register` and add `ToolRule::for_tool::<CurrentTimeTool>()` to `ALLOWED_TOOLS`.

## Acceptance criteria
- [x] A table test of the pure function with fixed `Zoned` values covers: a Singapore afternoon (the example above), midnight (`00:00`), 23:59, a date across a month and year end (2026-12-31 → 2027-01-01 at midnight), a zone with a negative and a half-hour offset (e.g. `America/St_Johns`), and `Etc/Unknown`.
- [x] The JSON keys are exactly the five above (a test on the serialized output).
- [x] The tool's name and parameters schema (empty object, `additionalProperties: false`) are tested; unknown arguments are rejected by the tool's `ToolRule`.
- [x] `registered_tools_match_the_allowed_list` passes with the new tool.
- [x] No `unwrap()`; `scripts/verify.sh` passes.
- [ ] On the Pi, "what time is it?" and "what day is it today?" by voice call `current_time` and the spoken answer matches the Pi's clock. (Pi check.)

## Comments
2026-11-21 (coordinator): built by a Sonnet agent in the main checkout; the coordinator read the whole diff, tightened `CurrentTime` and `format_current_time` to `pub(crate)`, and ran `scripts/verify.sh`: all checks passed. Integrated uncommitted in `dev`.
- **Code.** `src/tools/current_time.rs`: `CurrentTimeTool` (unit struct, `NAME` "current_time"), `format_current_time(&Zoned)` with named format constants (`%H:%M`, `%A`, `%Y-%m-%d`, `%:z`). jiff 0.2.37 facts read from its source: `Offset`'s `Display` prints `+08`, so the offset uses `%:z` (`+08:00`, `-03:30`, `+00:00`); `TimeZone::iana_name()` is `None` for the unknown zone and also for fixed-offset and POSIX zones, so each of those is reported as `Etc/Unknown` (the time and offset are still right).
- **Tests** (5 in `tests/unit/tools/current_time_tests.rs`): one table of 7 instants (Singapore 14:30, midnight, 23:59:59, 2026-12-31 23:59 and 2027-01-01 00:00, `America/St_Johns` at -03:30, `Etc/Unknown`); the exact JSON object; name and schema; the description; the rule accepts `{}` and rejects `{"zone":"UTC"}` and `null`. Spot-check by the agent: `%H.%M` made two tests fail; restored.
- **Existing tests changed** because four tools are now offered: the tool counts in `local_llm_tests.rs` and `assistant_tests.rs` (3 to 4), and in `policy_hook_tests.rs` the two hand-built agents gain the tool and the expected offered lists gain `current_time` last.
- **On the Pi (read-only):** `/etc/localtime` links to `/usr/share/zoneinfo/Asia/Singapore` and `TZ` is unset in `jarvis.service`, so jiff should report `Asia/Singapore` there. Not yet run on the Pi; the voice check is open.
