# 14: Storage on the stats Dashboard

Status: needs-info
Stories: US2
Blocked by: 10
Assignee: unassigned
Owns: `crates/orb-protocol/` (two Stats fields and `MAX_LINE_BYTES`); `src/features/stats/`; `ui/src/link/protocol.ts`; `ui/src/panels/`; `ui/src/preview/fixtures.ts`; and their tests
Traces to: D2, D7, D11

**Why:** Darren, 2026-11-16, after seeing the Dashboard on the Pi: "The dashboard looks good maybe it shld include the total storage too".

**What to build:**
- The Stats message gains the storage of the 8 GB Pi: bytes used and bytes in total, from the existing `sysinfo` dependency (`Disks`). `MAX_LINE_BYTES` is recomputed for the widest message, with its boundary test.
- The Dashboard shows it as "used / total GB", like memory, with a meter. It must still fit 800x480 with the widest values.
- The UI part goes to an Opus agent (the layout is judged by eye); the protocol and the stats source go to a Sonnet agent or the coordinator.

**Open questions for Darren (why this is `needs-info`):**
1. Which storage: the root filesystem `/` of the 8 GB Pi only (proposed), or also the 16 GB Pi? The 16 GB Pi has no stats service (spec, Out of Scope), so its storage would need one.
2. Used and total (proposed, "41.2 / 58.0 GB"), or only the total as he said?
3. GB as 2^30 bytes like the memory value and `df -h` (proposed), so a 64 GB card reads about 58?

## Acceptance criteria
- [ ] Storage used and total reach the UI in a Stats message; a Stats message without them is rejected by both decoders.
- [ ] `MAX_LINE_BYTES` equals the widest encodable Stats line; tested at that size and one byte over.
- [ ] The Dashboard shows storage to one decimal place; tested at 0 used and at used equal to total.
- [ ] A filesystem that cannot be read shows "Unavailable", never 0.
- [ ] The layout has no overflow at 800x480 with the widest values, by screenshot, then by eye on the Pi.
- [ ] `scripts/verify.sh` passes.

## Comments
