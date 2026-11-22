# 05: Timer messages in `orb-protocol` and the UI's protocol parser

Status: ready-for-agent
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
- [ ] Rust: the `timers` message and each of the three UI messages round-trip; unknown states, unknown fields, a missing `id`, a negative `id` and a non-integer `ends_at_ms` are rejected; an empty `timers` list and a timer with no label are accepted.
- [ ] TypeScript: the parser accepts the same valid messages and rejects the same invalid ones (the same table of cases as Rust, in `ui/tests/link/`).
- [ ] Existing protocol tests are unchanged and pass.
- [ ] No new crate or npm package; `scripts/verify.sh` passes (it runs `typecheck`, `lint` and `npm test` for the UI).

## Comments
