# The two Pis: facts for agents

Read this before any SSH, measurement or deployment step. Any change on a Pi needs Darren's OK first; read-only checks and builds in a scratch directory are fine.

## Hosts

| Host | Login | Role |
| --- | --- | --- |
| 8 GB Pi | `darrenrasp@jarvis` | Runs Jarvis (`~/jarvis`, branch `dev`), the microphone, the speaker, the 5 inch screen and the screen UI (Chromium kiosk). |
| 16 GB Pi | `raspdarren-ai@jarvis-ai` | Runs the models. Reachable only through the 8 GB Pi: `ssh -J darrenrasp@jarvis raspdarren-ai@jarvis-ai`. |

Both use password login; the password is `JARVIS_PASS` in the repo's `.env` on the Mac. Use the wrapper so it never appears on a command line:

```sh
export JARVIS_PASS="$(grep -E '^JARVIS_PASS=' .env | cut -d= -f2-)"
scripts/pi/ssh.exp ssh -o NumberOfPasswordPrompts=1 darrenrasp@jarvis 'uptime'
scripts/pi/ssh.exp scp -o NumberOfPasswordPrompts=1 file darrenrasp@jarvis:/tmp/
```

One password attempt per host per try, to avoid lockouts.

Things that went wrong with the wrapper on 2026-11-15:

- The remote command's first output line comes out on the same line as the password prompt. Strip the prompt (`sed 's/.*password: *//'`); filtering out lines that contain "password:" loses that line.
- The wrapper does not pass stdin on, so `ssh host 'python3 -' < script` hangs. Copy the script over with `scp` first, or send it inline as base64.
- `sudo` needs `ssh -t` and `sudo -p 'sudo password:'`: the wrapper only answers a prompt that ends in "assword:", and sudo's own prompt does not.
- The wrapper gives up after 300 s. Start anything longer with `setsid nohup ... &` and poll its log.
- `pkill -f <text>` over SSH also kills the shell running it, because the text is in that shell's command line.

## Services on the 16 GB Pi (as seen 2026-11-15)

| Port | What | Started as |
| --- | --- | --- |
| 8080 | Gemma (`unsloth/gemma-4-E2B-it-qat-GGUF:UD-Q4_K_XL`) | `llama-server ... -t 4 -c 8192 --parallel 2 -b 512 -ub 128 --jinja --temp 0 --reasoning off`; 2 slots of 4,096 tokens each (with an explicit `--parallel` the context is divided between the slots). Changed 2026-11-15 from `-c 2048` with 4 automatic slots; the old unit file is `~/jarvis-gemma.service.bak-2026-11-15` |
| 8081 | BGE embeddings (`bge-small-en-v1.5`) | `llama-server --embedding --ctx-size 512 --parallel 1` |
| 8082 | Speech-to-text (`whisper-medium` on the Hailo-10H) | `scripts/stt/jarvis-stt.service` |

Read-only endpoints that change nothing: `curl localhost:8080/props`, `curl localhost:8080/slots`, each server's `/health`.

Measured: Gemma reads about 58 prompt tokens a second, so every 58 uncached tokens cost a second before the first word. Speech-to-text takes about 2.0 s for a 2 s clip.

## Jarvis on the 8 GB Pi

- Since 2026-11-20 Jarvis runs as the systemd user service `jarvis.service` (repo: `scripts/pi/jarvis.service`): the **release** binary `target/release/jarvis`, `Restart=always`, enabled, with linger on so it starts at boot. The REPL exits at the end of stdin, so the unit holds stdin open with `tail -f /dev/null |`. After pulling code: `cargo build --release --locked && systemctl --user restart jarvis`. Log (with the timing lines): `journalctl --user-unit jarvis -f`. Before running `./run.sh` by hand (a **debug** build), `systemctl --user stop jarvis`: both want port 8765 and the microphone. Benchmarks also build into `target/release/`.
- The Orb is drawn by the screen UI (see "The screen UI" below), which reaches Jarvis on the WebSocket at 127.0.0.1:8765. The macroquad `jarvis-orb` and its Unix socket were removed by jarvis-ui ticket 06.
- Jarvis prints its timing lines (`Model stage: ... first_text=... cached_input=...`, `Speech diagnostics ... time_to_first_audio=...`) to the terminal only. To see a real run's timings, ask Darren to paste them.

## Measuring without touching Darren's data

Both run on the 8 GB Pi from `~/jarvis` and need only Python's standard library:

- `python3 scripts/pi/bench_first_text.py <binary> [--empty] [--pause=SECONDS] [prompts.txt]`: the wait for the first word per question, on a copy of the database. `--pause` leaves Jarvis idle between questions, which a cache warm-up needs. Use questions the server has not seen since its last restart: a repeated conversation is answered from the server's cache and looks faster than it is. The 2026-11-14 baseline is in `docs/specs/fast-memory/issues/01-search-only-on-demand.md`.
- `python3 scripts/pi/stt_timing.py`: speech-to-text latency.

To benchmark a change that is not pushed, build it beside the checkout instead of in it:

```sh
git worktree add --detach /tmp/jarvis-exp HEAD      # in ~/jarvis on the Pi
cd /tmp/jarvis-exp && git apply /tmp/change.patch
CARGO_TARGET_DIR=/tmp/jarvis-exp-target cargo build --release --locked --offline -p jarvis
# measure, then:
cd ~/jarvis && git worktree remove --force /tmp/jarvis-exp && rm -rf /tmp/jarvis-exp-target
```

Remove every scratch file when done and say in the ticket what was left behind.

## The screen UI (jarvis-ui): launch and deploy

Set up 2026-11-15 (jarvis-ui ticket 05). Chromium shows the React UI full screen on the 5 inch screen; Jarvis serves the WebSocket the UI talks to.

| Piece | Where | Detail |
| --- | --- | --- |
| Built UI | `~/jarvis/ui/dist` on the Pi | Copied from the Mac by `scripts/deploy-ui.sh` (`npm ci`, `npm run build`, `rsync --delete`). `ui/dist/` is git-ignored, so the checkout stays clean. The Pi has no Node. `scripts/deploy-ui.sh --dry-run` builds, then prints the copy commands. |
| UI server | `~/.config/systemd/user/jarvis-ui.service` (repo: `scripts/pi/jarvis-ui.service`) | `python3 -m http.server 8766 --bind 127.0.0.1 --directory %h/jarvis/ui/dist`, `Restart=on-failure`, enabled with `systemctl --user enable --now jarvis-ui`. Port 8766 is `DEFAULT_UI_PORT` in `src/config/orb.rs`; Jarvis's default allowed origin is `http://localhost:8766`. Log: `journalctl --user-unit jarvis-ui` (the access log; plain `-u` finds nothing for a user unit here). |
| Launcher | `~/.local/bin/jarvis-kiosk.sh` (repo: `scripts/pi/jarvis-kiosk.sh`, copied by the deploy script) | Waits up to 60 s for 127.0.0.1:8766 to accept a connection (a bare TCP connect, so it adds nothing to the access log), then `exec chromium --user-data-dir=$HOME/.config/jarvis-ui-chromium --password-store=basic --ozone-platform=wayland --kiosk --no-first-run --noerrdialogs --disable-infobars http://localhost:8766/`. `--password-store=basic` is required: without it Chromium 154 never sends the page request (`notes/ui-spike/http-hang-2026-11-15.md`). No `--allow-file-access-from-files`. |
| Autostart | `~/.config/labwc/autostart` (repo: `scripts/pi/labwc-autostart`) | labwc reads this file INSTEAD of `/etc/xdg/labwc/autostart`, so the file repeats that file's four lines (pcmanfm-pi, wf-panel-pi, kanshi, lxsession-xdg-autostart) and adds `$HOME/.local/bin/jarvis-kiosk.sh &`. If the system file changes, update the copy. |
| Cursor | `~/.local/share/icons/jarvis-blank/` and `XCURSOR_THEME=jarvis-blank` | labwc 0.20 has no setting that hides the pointer for good (the `HideCursor` action only hides it until the next pointer event). So `scripts/pi/make_blank_cursor.py` builds an Xcursor theme whose 62 cursors are one transparent pixel. The launcher exports `XCURSOR_THEME` for Chromium; a line in `~/.config/labwc/environment` makes labwc itself use it (read at labwc start, so it takes effect at the next login). |

Ports: `ss -ltn` shows `127.0.0.1:8766` (UI server) and `127.0.0.1:8765` (Jarvis WebSocket) only. Stopping Jarvis leaves 8766 up, and the page still loads.

Starting Jarvis over SSH so it keeps running with a terminal: `tmux new-session -d -s jarvis -c ~/jarvis 'export PATH=$HOME/.cargo/bin:$PATH XDG_RUNTIME_DIR=/run/user/1000 WAYLAND_DISPLAY=wayland-0; ./run.sh; echo EXITED; sleep 3600'` (`cargo` is not on the non-interactive SSH PATH). Attach with `tmux attach -t jarvis`; send a typed question with `tmux send-keys -t jarvis "question" Enter`.

Measuring the UI: start Chromium with the launcher's flags plus `--remote-debugging-port=9222`, read the frame rate with one `Runtime.evaluate` of a `requestAnimationFrame` counter over 30 s (a stdlib WebSocket client; `websockets` is not installed), and sum CPU ticks (`/proc/<pid>/stat`) and `Pss` (`/proc/<pid>/smaps_rollup`) over every `chromium` process in the same window, as `notes/ui-spike/measure.py` does. A screenshot: `XDG_RUNTIME_DIR=/run/user/1000 WAYLAND_DISPLAY=wayland-0 grim /tmp/x.png` (grim is installed; it does not draw the cursor).

### Removing each change

- Autostart: `rm ~/.config/labwc/autostart` (labwc falls back to the system file), then log out and in. Stop a running kiosk with `pkill -x chromium`.
- Cursor: delete the `XCURSOR_THEME=jarvis-blank` line from `~/.config/labwc/environment`, `rm -r ~/.local/share/icons/jarvis-blank`.
- UI server: `systemctl --user disable --now jarvis-ui; rm ~/.config/systemd/user/jarvis-ui.service; systemctl --user daemon-reload`.
- Files: `rm ~/.local/bin/jarvis-kiosk.sh; rm -r ~/jarvis/ui/dist ~/.config/jarvis-ui-chromium`.
