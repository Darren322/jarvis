# 05: Timer messages in `orb-protocol` and the UI's protocol parser

Status: done
Stories: US4, US5
Blocked by: None
Assignee: unassigned
Owns: `crates/orb-protocol/src/lib.rs`, `crates/orb-protocol/tests/**`, `ui/src/link/protocol.ts`, `ui/tests/link/**`
Traces to: D14

**What to build:** The screen contract of D14, on both sides of the wire, with no behaviour behind it yet.
- **To the UI:** `ToOrb::Timers { timers }`, wire `{"type":"timers","timers":[{"id":3,"label":"Time to sleep","ends_at_ms":…,"state":"running"}]}`. A timer has `id`, an optional `label`, `ends_at_ms` (unix milliseconds) and `state` (`pending`, `running`, `ended`). `deny_unknown_fields` as for the other messages. An empty list is a valid message (clears the strip).
- **From the UI:** `FromOrb` gains `cancel_timer`, `confirm_timer` and `reject_timer`, each `{"type":…,"id":3}`.
- **Line limits:** a label is at most `MAX_TIMER_LABEL_CHARS` characters (defined here, used by tickets 02 and 04), chosen so a `timers` message with 5 timers at the longest label fits `MAX_TO_ORB_LINE_BYTES`; the widest-line test covers it. The three UI messages must fit the inbound `MAX_LINE_BYTES` (274) at the largest id.
- Mirror both in `ui/src/link/protocol.ts` (the parser rejects unknown types, states and fields, as for `show_built_panel`) and its types. The UI strip itself is ticket 09.

## Acceptance criteria
- [x] Rust: the `timers` message and each of the three UI messages round-trip; unknown states, unknown fields, a missing `id`, a negative `id` and a non-integer `ends_at_ms` are rejected; an empty `timers` list and a timer with no label are accepted.
- [x] TypeScript: the parser accepts the same valid messages and rejects the same invalid ones (the same table of cases as Rust, in `ui/tests/link/`).
- [x] Existing protocol tests are unchanged and pass.
- [x] No new crate or npm package; `scripts/verify.sh` passes (it runs `typecheck`, `lint` and `npm test` for the UI).

## Comments
- 2026-11-22 (coordinator): built by a Sonnet agent in a worktree from `d6b5f08`; reviewed and integrated into `dev`. Added to `orb-protocol`: `MAX_TIMER_LABEL_CHARS = 60` (characters, not bytes), `MAX_TIMERS = 5`, `TimerState` (`pending`, `running`, `ended`; cancelled timers are absent), `Timer` (checked label; `id` and `ends_at_ms` are `u64`, so negatives and non-integers never decode; a `null` label is invalid and no label means the key is absent), `ToOrb::Timers`, and `FromOrb::{CancelTimer, ConfirmTimer, RejectTimer} { id }`. `MAX_TO_ORB_LINE_BYTES` is now the larger of the widest built Panel and the widest `timers` line (2,283 bytes), so its value is unchanged. The inbound messages fit 274 bytes at `u64::MAX`. The TS parser mirrors the same case table and accepts only safe integers. Its one gap: a float written `1.0` parses in JS, since `JSON.parse` cannot tell it from `1`. Spot-check (agent): `MAX_TIMER_LABEL_CHARS` set to 59 failed two tests; restored. Coordinator: `scripts/verify.sh` in the main checkout, all checks passed (7 new Rust tests, 403 Vitest tests). No arms were needed outside the owned files. Nothing here needs the Pi.
