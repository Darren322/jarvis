# 16: Jarvis can say the stats it shows

Status: ready-for-agent
Stories: US6
Blocked by: 14
Assignee: unassigned
Owns: `src/tools/system_status.rs`, `src/tools/system_status_tool.rs`, `src/tools/mod.rs` (only the `.tool(SystemStatusTool)` line, which becomes a constructor once the tool holds a `SystemMachine`), `src/features/stats/machine.rs` (only if the reading is shared from there), and their tests in `tests/unit/tools/` and `tests/unit/features/stats/`
Traces to: D2, D7

**Why:** On 2026-11-20, with the Dashboard on screen, Darren asked by voice "I mean how much storage do I have?" and Jarvis answered "I do not have access to information regarding your personal storage, master. I can only report on the system's operational status." Darren: "i want jarvis to see the stats". He agreed to read the stats, not the UI ("Okay sure lets do it").

**Cause, read in the code on 2026-11-20:** the model already has a tool, `system_status`, but it reports only hostname, uptime and memory in raw bytes, from its own collector (`collect_system_status` in `src/tools/system_status.rs`). The Dashboard reads through a second collector, `SystemMachine` in `src/features/stats/machine.rs`. The tool's description mentions neither storage nor CPU, so Gemma never called it.

**What to build:**
- `system_status` returns the figures the Dashboard shows for the 8 GB Pi: CPU, temperature, memory and storage, plus the hostname and uptime it has today. Its numbers come from `SystemMachine`, and `collect_system_status`'s separate `sysinfo` reading is removed, so the screen and the answer cannot disagree.
- Values are given in the Dashboard's units, so Gemma never does arithmetic on numbers like `251416899584`.
- The tool description names every figure (storage, disk space, CPU, temperature, memory, uptime), so a question about any of them leads to the tool.
- The tool keeps the name `system_status`: `tests/unit/clients/local_llm_tests.rs` uses it on the wire, and history may hold calls to it.
- A tool reading from `features::stats` has a precedent: `show_panel` already uses `features::panel`.

**Proposals for Darren to overrule** (the coordinator's choices, 2026-11-20):
- **P1. Units match the screen.** Memory as GB of 2^30 bytes, to one decimal place (the Dashboard's `formatRamGb`). Storage as whole GB of 10^9 bytes, with used = total minus available (`formatStorageGb`, ticket 14). CPU as a whole percent and temperature as whole °C. The rounding rules are written twice, in TypeScript and in Rust, so both are tested at the same boundaries.
- **P2. CPU needs a warm-up.** `sysinfo` measures CPU as the difference between two refreshes at least `sysinfo::MINIMUM_CPU_UPDATE_INTERVAL` (200 ms) apart. The tool takes a reading, waits that long under a named constant, and reads again, so the figure covers the last 200 ms. That adds 200 ms to the tool call. The alternative is to share the stats source's last reading, but that reading is fresh only while the Dashboard is shown.
- **P3. No llama or BGE health.** Those probes run only while the Dashboard is shown, and a fresh check from the tool would mean a network wait inside a model run. If Jarvis can answer at all, llama is up. Health can be a later ticket if Darren wants it.
- **P4. Missing figures say so.** A temperature or storage figure that cannot be read is reported as `"unavailable"`, never 0 (AGENTS: never report a made-up value as fact). If the whole reading fails (`MachineError`), every machine figure is unavailable and hostname and uptime are still given, so the tool's error type stays `Infallible` and the model can say it could not read the Pi.

## Acceptance criteria
- [ ] `system_status` returns hostname, uptime, CPU percent, temperature, memory used and total, and storage used and total, in the units of P1. Tested with fixed readings.
- [ ] Memory and storage figures equal the Dashboard's for the same bytes. Tested at ticket 14's rounding boundary (half a GB of storage rounds up) and at 0 used and nothing available, in a table.
- [ ] A temperature or storage figure that cannot be read appears as unavailable, never 0, and a failed reading makes every machine figure unavailable. Tested with a scripted `Machine`.
- [ ] `collect_system_status`'s own `sysinfo` reading is gone; the figures come from `SystemMachine` (or one reading both share).
- [ ] The tool's output for the widest values is stated in bytes and tokens in Comments, and stays under 100 tokens.
- [ ] The description names storage, CPU, temperature, memory and uptime. `registered_tools_match_the_allowed_list` still passes.
- [ ] On the Pi, with the Dashboard shown, "how much storage do I have?" by voice calls `system_status`, and the spoken figure matches the screen. (Pi check.)
- [ ] `scripts/verify.sh` passes.

## Comments
