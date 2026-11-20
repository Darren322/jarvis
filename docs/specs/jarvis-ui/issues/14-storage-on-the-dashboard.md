# 14: Storage on the stats Dashboard

Status: done
Stories: US2
Blocked by: 10
Assignee: Sonnet agent (protocol and stats source), Opus agent (UI), 2026-11-16
Owns: `crates/orb-protocol/` (two Stats fields and `MAX_LINE_BYTES`); `src/features/stats/`; `ui/src/link/protocol.ts`; `ui/src/panels/`; `ui/src/preview/fixtures.ts`; and their tests
Traces to: D2, D7, D11

**Why:** Darren, 2026-11-16, after seeing the Dashboard on the Pi: "The dashboard looks good maybe it shld include the total storage too".

**What to build:**
- The Stats message gains the storage of the 8 GB Pi: bytes used and bytes in total, from the existing `sysinfo` dependency (`Disks`). `MAX_LINE_BYTES` is recomputed for the widest message, with its boundary test.
- The Dashboard shows it as "used / total GB", like memory, with a meter. It must still fit 800x480 with the widest values.
- The UI part goes to an Opus agent (the layout is judged by eye); the protocol and the stats source go to a Sonnet agent or the coordinator.

**Settled 2026-11-16** (Darren: "sure lets go with the rest" to the coordinator's proposals; the unit is the coordinator's choice after his "i thought its 128gb?", for him to overrule):
- **Which storage.** The root filesystem `/` of the 8 GB Pi only. Read on 2026-11-16: it is a 256 GB Samsung NVMe drive (`df`: 235G in size, 32G used, 194G available). The 16 GB Pi has a 128 GB SD card, but no stats service, so it is not shown.
- **On the wire.** Stats gains an optional object, placed after `ram_total_bytes`: `"storage":{"available_bytes":207517036544,"total_bytes":251416899584}`. It is absent when the filesystem cannot be read. Both numbers are facts from `sysinfo` (`Disk::available_space`, `Disk::total_space`); the UI works out what is used.
- **On the Dashboard.** "used / total GB" with a meter, where used is total minus available. That counts the blocks the filesystem keeps in reserve as used, so it reads higher than `df`'s Used column (about 44 against 34 on the Pi today).
- **Unit.** Whole GB of 10^9 bytes, the unit drives are sold in, so the 256 GB drive reads "44 / 251 GB". Memory stays in 2^30 bytes (the 8 GB Pi reads 7.9). A missing `storage` shows "Unavailable".

## Acceptance criteria
- [x] Storage available and total reach the UI in a Stats message; a Stats message without `storage` is accepted and shows "Unavailable"; a `storage` object with a missing or extra key is rejected by both decoders.
- [x] `MAX_LINE_BYTES` equals the widest encodable Stats line; tested at that size and one byte over.
- [x] The Dashboard shows storage in whole GB of 10^9 bytes; tested at 0 used (available equal to total), at nothing available, and at the rounding boundary.
- [x] A filesystem that cannot be read shows "Unavailable", never 0.
- [x] The layout has no overflow at 800x480 with the widest values, by screenshot, then by eye on the Pi. (Screenshots done; the Pi screenshot of real values fits; Darren's eye not yet.)
- [x] `scripts/verify.sh` passes.

## Comments
- 2026-11-17 (coordinator): both halves integrated uncommitted into `dev`; worktrees removed.
  - **Rust (Sonnet agent).** `orb_protocol::StorageUsage { available_bytes, total_bytes }` (`deny_unknown_fields`); `ToOrb::Stats` gains `storage: Option<StorageUsage>`, absent when unread, and a present `null` is rejected (`present_storage`). `MAX_LINE_BYTES` 188 to 274, the widest line with `u64::MAX` storage; spot-checked at 273 and 275. `machine.rs` lists disks once and each second refreshes only the disk mounted at `STORAGE_MOUNT_POINT` (`/`), storage figures only, inside the existing `spawn_blocking`; a disk reporting 0 total is `None`, never zeros.
  - **UI (Opus agent; stopped once by the usage limit and resumed).** `StorageSpace` in `protocol.ts`, parsed as strictly as the Rust side. `format.ts`: `STORAGE_BYTES_PER_GB` = 10^9, whole GB, half rounds up; `BYTES_PER_GB` and `formatGb` renamed `RAM_BYTES_PER_GB` and `formatRamGb` so the two units cannot be confused. Storage sits right of Memory on the second row; that row's figures went from 68 to 52 px (units 28 to 20 px) so "9999 / 9999 GB" fits one column (Verdana: about 5 px to spare). CPU, temperature and the services are unchanged. UI tests 220 to 270; five boundaries broken by one and seen to fail.
  - **Seen by the coordinator.** `notes/ui-spike/dashboard-shots/storage-typical.png` ("2.3 / 7.9 GB" beside "44 / 251 GB") and `storage-widest-verdana.png`: no overflow.
  - **Verified in the main checkout.** `scripts/verify.sh`: all checks passed (453 root unit tests, 15 protocol tests, 270 UI tests); `npm run build` passes.
  - **Deploy together.** The UI on the Pi rejects a Stats message with an unknown key, so Jarvis with this change and the UI with it must reach the Pi together, or the Dashboard waits forever.
  - **Not verified.** Anything on the Pi: the real figures against `df -B1 /`, the Pi's font, the 52 px row from a metre.
- 2026-11-17 (coordinator): `dev` pushed at Darren's request and deployed to the 8 GB Pi in one step: `git pull` to `d26da31`, `cargo build` (21 s), `scripts/deploy-ui.sh`, Jarvis restarted in tmux with `./run.sh`, Chromium restarted with the kiosk launcher. Checked: the screen reconnected (one WebSocket client); typing "bring up the stats" printed "Showing the stats." with no model run (15); a `grim` screenshot (`notes/ui-spike/dashboard-shots/pi-storage-2026-11-17.png`) shows CPU 6%, 55 C, 0.9 / 7.9 GB, Storage 44 / 251 GB, llama and BGE up, nothing cut off (14). `df -B1 /` at the same time: 251,416,899,584 total, 207,441,403,904 available, so 44 GB used, matching. Caution: Jarvis's last line before the restart was `[thinking]`, so a question Darren was asking at that moment may have been cut off. Still for Darren: closing in his own words (13), and how quick answers are after a tool call (fast-memory 10).
- 2026-11-20 (coordinator): the by-eye check is Darren's report, not a log: "items 1 to 5 are ok" (item 3 was the storage figure fitting on the screen). Done.
