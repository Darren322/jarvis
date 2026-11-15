# 05: Kiosk launch and deploy

Status: in-progress
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
- [ ] Darren's OK for each Pi change is recorded in Comments.
- [ ] `bash -n scripts/deploy-ui.sh` passes. The dry run prints the commands and changes nothing.
- [ ] A failed `npm run build` stops the script before `rsync` runs. Checked by a dry run with the build forced to fail.
- [ ] After login, Chromium opens full screen with no browser chrome. The cursor is hidden. (Pi check.)
- [ ] With Jarvis stopped, the offline Orb shows and taps are ignored. Starting Jarvis connects within `RECONNECT_DELAY` plus a few seconds. (Pi check.)
- [ ] On the Pi, Darren taps while idle, speaks, stops talking and hears the answer, with no key pressed. (Pi check.)
- [ ] The Orb shows every state on the Pi. Taps behave as in visual-jarvis 10. (Pi check.)
- [ ] Frame rate, RAM and CPU are measured on the Pi while Jarvis speaks, and compared with D11. The method is written in the report.
- [ ] A reboot brings the Orb back. (Pi check.)
- [ ] Five launches in a row each reach `GET /` in the UI server's log. (Pi check.)
- [ ] `ss -ltn` on the Pi shows the UI server listening on 127.0.0.1 only. Stopping Jarvis leaves the page served. (Pi check.)
- [ ] The removal steps for the autostart entry are in `docs/agents/pi.md`.
- [ ] The report says that the Pi checks are not proven by local tests.

## Comments

- 2026-11-15 (coordinator): A Sonnet agent diagnosed the white-screen hang (`notes/ui-spike/http-hang-2026-11-15.md`, harness `http-hang-test.sh`). It is not python's http.server: with a normal on-disk profile Chromium 154 never sends the main request to any server (0 of 6). `--password-store=basic` fixed it every time (5 of 5 with fresh profiles, 6 of 6 with a reused profile on localhost:8766, HTTP/1.0 and 1.1), as did `--incognito` (which loses storage). The likely cause, unproven, is the network service waiting on gnome-keyring for the cookie key. Recommended launcher: `chromium --user-data-dir=$HOME/.config/jarvis-ui-chromium --password-store=basic --ozone-platform=wayland --kiosk --no-first-run --noerrdialogs --disable-infobars http://localhost:8766/`. Not tested: Chromium starting before the server.
- 2026-11-15 (coordinator): Darren's OK for every Pi change in this ticket (Q37, "yes to all"): push `dev` and rebuild Jarvis on the 8 GB Pi; copy `ui/dist/`; add and enable the `jarvis-ui.service` user service on 127.0.0.1:8766; create `~/.config/labwc/autostart` for Chromium kiosk with `--password-store=basic` and the cursor hidden; measure CPU, RAM and fps per state.
