# 05: Kiosk launch and deploy

Status: done
Stories: US1
Blocked by: 03, 04
Assignee: Sonnet agent (2026-11-15)
Owns: `scripts/deploy-ui.sh`; `scripts/pi/jarvis-ui.service`; the launch section of `docs/agents/pi.md` only
Traces to: D1, D4, D6, D11

**What to build:**
- `scripts/deploy-ui.sh` builds `ui/dist` on the Mac (`npm ci`, then `npm run build`) and copies it to the 8 GB Pi with `rsync`, through the host setup in `docs/agents/pi.md`. The script uses `set -euo pipefail` and stops at the first failure. It has a dry-run mode that prints the `rsync` command without running it.
- On the Pi, a systemd user service (for example `jarvis-ui.service`, file kept in the repo under `scripts/pi/` and copied by the deploy script) runs `python3 -m http.server <port> --bind 127.0.0.1 --directory <deployed ui/dist>` and restarts on failure (D4). The port is a named setting, recorded in `docs/agents/pi.md`.
- A labwc autostart entry in `~/.config/labwc/autostart` (the file does not exist yet) starts Chromium in kiosk mode on `http://localhost:<port>`. It does not use `--allow-file-access-from-files`. It uses its own profile and `--password-store=basic` (without it, Chromium 154 on the Pi never sends the page request: see the 2026-11-15 comment). It waits for the UI server or retries if Chromium starts first. The cursor is hidden. The agent reports the method it used. `notes/ui-spike/run_chromium.sh` is the starting point.
- The launch section of `docs/agents/pi.md` records the deployed path, the Chromium flags, how to hide the cursor, and how to remove the autostart entry.

**Waiting on:** Darren's OK for each Pi change (D6, push-to-talk D10). Before changing the Pi, the agent lists exactly what it will change, and then does it. The Pi needs no Node.

## Acceptance criteria
- [x] Darren's OK for each Pi change is recorded in Comments.
- [x] `bash -n scripts/deploy-ui.sh` passes. The dry run prints the commands and changes nothing.
- [x] A failed `npm run build` stops the script before `rsync` runs. Checked by a dry run with the build forced to fail.
- [ ] After login, Chromium opens full screen with no browser chrome. The cursor is hidden. (Pi check.)
- [x] With Jarvis stopped, the offline Orb shows and taps are ignored. Starting Jarvis connects within `RECONNECT_DELAY` plus a few seconds. (Pi check.)
- [x] On the Pi, Darren taps while idle, speaks, stops talking and hears the answer, with no key pressed. (Pi check.)
- [x] The Orb shows every state on the Pi. Taps behave as in visual-jarvis 10. (Pi check.)
- [x] Frame rate, RAM and CPU are measured on the Pi while Jarvis speaks, and compared with D11. The method is written in the report.
- [ ] A reboot brings the Orb back. (Pi check.)
- [x] Five launches in a row each reach `GET /` in the UI server's log. (Pi check.)
- [x] `ss -ltn` on the Pi shows the UI server listening on 127.0.0.1 only. Stopping Jarvis leaves the page served. (Pi check.)
- [x] The removal steps for the autostart entry are in `docs/agents/pi.md`.
- [x] The report says that the Pi checks are not proven by local tests.

## Comments

- 2026-11-15 (coordinator): A Sonnet agent diagnosed the white-screen hang (`notes/ui-spike/http-hang-2026-11-15.md`, harness `http-hang-test.sh`). It is not python's http.server: with a normal on-disk profile Chromium 154 never sends the main request to any server (0 of 6). `--password-store=basic` fixed it every time (5 of 5 with fresh profiles, 6 of 6 with a reused profile on localhost:8766, HTTP/1.0 and 1.1), as did `--incognito` (which loses storage). The likely cause, unproven, is the network service waiting on gnome-keyring for the cookie key. Recommended launcher: `chromium --user-data-dir=$HOME/.config/jarvis-ui-chromium --password-store=basic --ozone-platform=wayland --kiosk --no-first-run --noerrdialogs --disable-infobars http://localhost:8766/`. Not tested: Chromium starting before the server.
- 2026-11-15 (coordinator): Darren's OK for every Pi change in this ticket (Q37, "yes to all"): push `dev` and rebuild Jarvis on the 8 GB Pi; copy `ui/dist/`; add and enable the `jarvis-ui.service` user service on 127.0.0.1:8766; create `~/.config/labwc/autostart` for Chromium kiosk with `--password-store=basic` and the cursor hidden; measure CPU, RAM and fps per state.
- 2026-11-15 (coordinator): Sonnet agent deployed to the 8 GB Pi: pulled 868d7d3 and rebuilt Jarvis (debug, tmux session `jarvis`); `scripts/deploy-ui.sh` copied `ui/dist`, `jarvis-ui.service` (enabled, 127.0.0.1:8766) and `~/.local/bin/jarvis-kiosk.sh` (waits for the server, then Chromium kiosk with `--password-store=basic`); `~/.config/labwc/autostart` repeats the four system autostart lines and adds the launcher; the cursor is hidden by a transparent Xcursor theme (`scripts/pi/make_blank_cursor.py`, `XCURSOR_THEME=jarvis-blank` in the launcher and `~/.config/labwc/environment`). Checked on the Pi: page connects (state idle reaches it), 5 of 5 launches reach `GET /`, the launcher waits if the server is down, only 127.0.0.1:8765 and :8766 listen, `/exit` shows the offline orb within 0.5 s and it reconnects after Jarvis restarts. Measured (Chromium, % of one core / PSS / fps): idle 16.7 / 464 MB / 60; speaking 30 to 39 / 466 / 60; listening 30 to 44 / 466 / 60; thinking 32 / 505 / 60; offline 16 / 505 / 60. Within D11 except PSS reaching 505 MB after about 10 minutes: soak it. Still for Darren: reboot (autostart, hidden cursor, desktop panel intact), the look on the screen, a real touch tap, and the full tap, speak and hear flow. The Pi runs 868d7d3, without 08 and 09 (pull and rebuild for those).
- 2026-11-16 (coordinator): Memory soak answered by one read-only reading instead of a one-hour run: the same Chromium has been up 82,752 s (about 23 hours, Jarvis connected and idle, never rebooted) and its 10 processes sum to 510 MB PSS (`/proc/<pid>/smaps_rollup`), against 505 MB after about 10 minutes on 2026-11-15. No growth worth acting on; 10 MB over D11's "about 500 MB". The Pi has 6.9 GB available. Not covered: memory while states change often, and with the Dashboard open (ticket 10).
- 2026-11-16 (coordinator): With Darren's OK ("Yep, its all good" to the pull and rebuild question), the 8 GB Pi was pulled to `b2d1a54` and Jarvis rebuilt (debug, no dependency or schema change since `868d7d3`) and restarted in tmux session `jarvis`. Checked: it listens on 127.0.0.1:8765 and the kiosk page reconnected (one established connection on that port). It now has the stats source and `show_panel` (08, 09). Not tried: "bring up the stats" by voice, and the UI on the Pi cannot draw the Dashboard until ticket 10 is deployed. Scratch file `/tmp/jarvis-build.log` removed. The Pi was not rebooted; Darren's checks are still open.
- 2026-11-16 (coordinator): Darren, after using it on the Pi: "Ticket 5 is good". Jarvis's terminal on the Pi shows two full runs after the 21:24 restart, each listening, transcribing, thinking, speaking, idle, with no typed input (one of them a two-turn tool run). Ticked on that: tap, speak and hear with no key; every state on the screen. Left unticked because nobody has observed them: "After login, Chromium opens full screen ... cursor hidden" and "A reboot brings the Orb back". The Pi has not rebooted or logged in again since the autostart was set up (uptime 1 day 26 min at 21:27, the same Chromium process since 2026-11-15), so the autostart entry and the labwc cursor setting have never run from a fresh login. Closed on Darren's word; the next reboot is the test, and Jarvis itself is not autostarted (it runs in tmux and must be started by hand after a reboot). Commit `907bf89`. Done.
