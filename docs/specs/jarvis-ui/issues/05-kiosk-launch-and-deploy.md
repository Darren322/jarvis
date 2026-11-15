# 05: Kiosk launch and deploy

Status: ready-for-agent
Stories: US1
Blocked by: 03, 04
Assignee: unassigned
Owns: `scripts/deploy-ui.sh`; `scripts/pi/jarvis-ui.service`; the launch section of `docs/agents/pi.md` only
Traces to: D1, D4, D6, D11

**What to build:**
- `scripts/deploy-ui.sh` builds `ui/dist` on the Mac (`npm ci`, then `npm run build`) and copies it to the 8 GB Pi with `rsync`, through the host setup in `docs/agents/pi.md`. The script uses `set -euo pipefail` and stops at the first failure. It has a dry-run mode that prints the `rsync` command without running it.
- On the Pi, a systemd user service (for example `jarvis-ui.service`, file kept in the repo under `scripts/pi/` and copied by the deploy script) runs `python3 -m http.server <port> --bind 127.0.0.1 --directory <deployed ui/dist>` and restarts on failure (D4). The port is a named setting, recorded in `docs/agents/pi.md`.
- A labwc autostart entry in `~/.config/labwc/autostart` (the file does not exist yet) starts Chromium in kiosk mode on `http://localhost:<port>`. It does not use `--allow-file-access-from-files`. The cursor is hidden. The agent reports the method it used. `notes/ui-spike/run_chromium.sh` is the starting point.
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
- [ ] `ss -ltn` on the Pi shows the UI server listening on 127.0.0.1 only. Stopping Jarvis leaves the page served. (Pi check.)
- [ ] The removal steps for the autostart entry are in `docs/agents/pi.md`.
- [ ] The report says that the Pi checks are not proven by local tests.

## Comments
