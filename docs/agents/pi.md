# The two Pis: facts for agents

Read this before any SSH, measurement or deployment step. Any change on a Pi needs Darren's OK first; read-only checks and builds in a scratch directory are fine.

## Hosts

| Host | Login | Role |
| --- | --- | --- |
| 8 GB Pi | `darrenrasp@jarvis` | Runs Jarvis (`~/jarvis`, branch `dev`), the microphone, the speaker, the 5 inch screen and `jarvis-orb`. |
| 16 GB Pi | `raspdarren-ai@jarvis-ai` | Runs the models. Reachable only through the 8 GB Pi: `ssh -J darrenrasp@jarvis raspdarren-ai@jarvis-ai`. |

Both use password login; the password is `JARVIS_PASS` in the repo's `.env` on the Mac. Use the wrapper so it never appears on a command line:

```sh
export JARVIS_PASS="$(grep -E '^JARVIS_PASS=' .env | cut -d= -f2-)"
scripts/pi/ssh.exp ssh -o NumberOfPasswordPrompts=1 darrenrasp@jarvis 'uptime'
scripts/pi/ssh.exp scp -o NumberOfPasswordPrompts=1 file darrenrasp@jarvis:/tmp/
```

One password attempt per host per try, to avoid lockouts.

## Services on the 16 GB Pi (as seen 2026-11-14)

| Port | What | Started as |
| --- | --- | --- |
| 8080 | Gemma (`unsloth/gemma-4-E2B-it-qat-GGUF:UD-Q4_K_XL`) | `llama-server ... -t 4 -c 2048 -b 512 -ub 128 --jinja --temp 0 --reasoning off`; 4 slots, each with the full 2,048-token context |
| 8081 | BGE embeddings (`bge-small-en-v1.5`) | `llama-server --embedding --ctx-size 512 --parallel 1` |
| 8082 | Speech-to-text (`whisper-medium` on the Hailo-10H) | `scripts/stt/jarvis-stt.service` |

Read-only endpoints that change nothing: `curl localhost:8080/props`, `curl localhost:8080/slots`, each server's `/health`.

Measured: Gemma reads about 58 prompt tokens a second, so every 58 uncached tokens cost a second before the first word. Speech-to-text takes about 2.0 s for a 2 s clip.

## Jarvis on the 8 GB Pi

- Darren starts it with `./run.sh`, which runs `cargo check --locked` and then `cargo run --locked`: a **debug** build. `target/release/` is only used by `jarvis-orb` and by benchmarks.
- The orb: `./target/release/jarvis-orb` (linked to Jarvis by default; `--demo` for the stand-alone demo). It needs `XDG_RUNTIME_DIR=/run/user/1000` and `WAYLAND_DISPLAY=wayland-0` when started over SSH, and it takes over the screen.
- The orb socket is `/run/user/1000/jarvis-orb.sock`.
- Jarvis prints its timing lines (`Model stage: ... first_text=... cached_input=...`, `Speech diagnostics ... time_to_first_audio=...`) to the terminal only. To see a real run's timings, ask Darren to paste them.

## Measuring without touching Darren's data

All three run on the 8 GB Pi from `~/jarvis` and need only Python's standard library:

- `python3 scripts/pi/bench_first_text.py <binary> [--empty] [prompts.txt]`: the wait for the first word per question, on a copy of the database. The 2026-11-14 baseline is in `docs/specs/fast-memory/issues/01-search-only-on-demand.md`.
- `python3 scripts/pi/orb_smoke.py`: the orb socket, a tap and mic levels against the real Jarvis; sends nothing to the model.
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
