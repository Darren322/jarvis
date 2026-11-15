# 07: Panel messages

Status: done
Stories: US2, US3
Blocked by: None
Assignee: Sonnet agent (2026-11-15)
Owns: `crates/orb-protocol/` (all files, including its `Cargo.toml`); `crates/jarvis-orb/src/` (only the match arms a new variant needs, until ticket 06 deletes the crate)
Traces to: D3, D7, D8

**Notes for the coordinator:**
- Adding variants can break `src/interfaces/orb/` (ticket 03) if it matches exhaustively. The coordinator reconciles that at integration. Ticket 03 is not edited here.

**What to build:** Extend `orb-protocol` with:
- ToOrb `ShowPanel { panel }`, where `panel` is a typed enum with one variant, `Stats`. Camera and Catalog variants come later, in their own tickets.
- ToOrb `HidePanel`.
- ToOrb `Stats`, with CPU percent (`f32`), temperature (optional, so that a missing sensor is absent and never zero), RAM used and total in bytes, and llama and BGE as up or down.
- FromOrb `PanelClosed`. This is **proposal P1** (approved 2026-11-15): the UI sends it when Darren taps outside the Panel.

`MAX_LINE_BYTES` is 256 today. It must be re-justified for the Stats message. Its doc comment states the size of the largest Stats message with the widest value for each field. A test computes that size.

## Acceptance criteria
- [x] Each new message round-trips through serde with its exact JSON shape, tested in a table.
- [x] Each serialized message contains no `\n`.
- [x] `MAX_LINE_BYTES` equals the size of the largest Stats message with the widest value for each field (a test computes it). Its doc comment says why.
- [x] A line of exactly `MAX_LINE_BYTES` bytes is accepted. A line of `MAX_LINE_BYTES + 1` bytes is rejected.
- [x] An unknown Panel name is rejected. An extra field is rejected (`#[serde(deny_unknown_fields)]`).
- [x] A missing temperature round-trips as absent, not as 0.
- [x] `crates/jarvis-orb` still compiles. Only the match arms needed for new variants change, and the change is listed in Comments.
- [x] `cargo test --locked --offline -p orb-protocol` passes. `scripts/verify.sh` passes.

## Comments

- 2026-11-15 (coordinator): Integrated uncommitted into `dev` from the agent's worktree (removed). Adds `ShowPanel { panel: PanelName }`, `HidePanel {}`, `Stats { cpu_percent: CpuPercent, temperature_c: Option<f32>, ram_used_bytes, ram_total_bytes, llama, bge: ServiceHealth }` and `FromOrb::PanelClosed {}`. `CpuPercent` is validated 0 to 100 like `Level`. `MAX_LINE_BYTES` is now 188, the computed widest Stats line (was 256); spot-checked at 187 and 189. jarvis-orb ignores the panel messages in one arm in `link.rs`. The root crate needed no new arm because ticket 03's dispatch routes every non-Tap message to `OrbHandle.from_orb`; a WebSocket test shows `PanelClosed` arriving there. Verified together with 03 (see 03's comment). Note for ticket 12: Catalog Panels will need a larger limit.
