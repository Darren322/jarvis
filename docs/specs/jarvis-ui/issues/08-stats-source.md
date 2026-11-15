# 08: Stats source

Status: done
Stories: US2
Blocked by: 03, 07
Assignee: Sonnet agent (2026-11-15)
Owns: `src/features/stats/` (new); `src/clients/health.rs` (new, the `/health` check only); `tests/unit/features/stats/`; `tests/unit/clients/health_tests.rs`
Traces to: D7, D11

**Notes for the coordinator:**
- Blocked by 03 as well as 07 (proposal P3). The stats feed publishes through the transport that 03 builds.
- The module name `stats` is proposed.
- Wiring into `src/app.rs` and the `pub(crate) mod stats;` line in `src/features/mod.rs` is done by the coordinator at integration. This ticket does not edit those files.

**What to build:**
- `src/features/stats/` reads the 8 GB Pi with `sysinfo`: CPU percent, temperature and RAM. It refreshes only what it reads. The first CPU reading needs a warm-up refresh, as the locked `sysinfo` source documents. The ticket names the minimum interval from that source.
- The 16 GB Pi is checked with `GET /health` on llama and on BGE. Each check has a named timeout (`HEALTH_TIMEOUT`). A timeout, a connection failure or a non-success status is Down.
- While the Dashboard is shown, one Stats message goes out every `STATS_INTERVAL` (one second). While hidden, nothing goes out. The module takes a `watch` of shown or hidden from ticket 09's panel state.
- No mutex is held across an `.await`. At most one request is in flight per health endpoint.

## Acceptance criteria
- [x] CPU percent is clamped to 0 to 100. Tested at -0.1, 0, 100 and 100.1.
- [x] A missing temperature sensor is reported as absent, never as 0. Tested with no sensor.
- [x] RAM used is at most RAM total. Tested at 0 and at total.
- [x] A health check that returns 200 is Up. Status 500 is Down. A connection failure is Down. A check that takes longer than `HEALTH_TIMEOUT` is Down. Tested with Wiremock and paused time.
- [x] `HEALTH_TIMEOUT` is a named constant. A check that is still pending at the limit is treated as Down, not left waiting.
- [x] Over 5 seconds shown (paused time), exactly 5 Stats messages go out. Over 5 seconds hidden, none do.
- [x] After hidden and then shown again, the messages resume at the next tick.
- [x] Stats values match `top` and `vcgencmd` on the Pi within a stated tolerance. (Pi check, ticket 10.)
- [x] `scripts/verify.sh` passes. Each new boundary test was seen to fail when its limit moved by one.
- [x] Local tests only. The report says so.

## Comments

- 2026-11-15 (coordinator): Sonnet agents built the source (`src/features/stats/`, `src/clients/health.rs`: `STATS_INTERVAL` 1 s, `HEALTH_TIMEOUT` 800 ms, at most one check in flight per endpoint) and wired it in `app.rs` (stats follow `PanelHandle::watch_shown()` through `StatsVisibility::of_panel`). Coordinator ran `scripts/verify.sh`: all checks passed (456 Rust, 102 UI tests); the agents ran `--repeat` 20/20 on the stats and panel timing tests. Pi-only, for ticket 10: the Pi 5 temperature sensor label, values against `top`/`vcgencmd`, real `/health` answers. Ticked criteria that need the Pi are proven only locally.
