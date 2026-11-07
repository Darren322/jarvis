# Spec: push-to-talk (roadmap 15A, thin slice)

Settled with Darren in a grilling session on 2026-11-06. This file is the source of truth for the work in progress. **Update the Status section at the end of every session** so the next session can resume from it.

## Status

| Ticket | State | Notes |
| --- | --- | --- |
| T0 16 GB Pi services | done | Installed 2026-11-06: `jarvis-gemma` and `jarvis-embeddings` systemd units, enabled; `/health` 200 from the 8 GB Pi after install and after restart (re-checked by Claude); BGE alias served. Wi-Fi power save off on both Pis. Reboot on 2026-11-07 (Hailo runtime install) proved boot start; Gemma /health 200 about 3 min after boot. `NetworkManager-wait-online` failed at boot (harmless so far; unknown whether it predates this) |
| T1 8 GB Pi cleanup + acceptance | in review | Cleanup done and re-checked 2026-11-06. By ear: speech plays, `/stop` works; long answers fixed by real-time speech (closed 2026-11-07). 2026-11-07: recall works, and `/memories <text>` and `/forget <id>` worked on the Pi. Remaining: `/memory status` shows the embedder ready |
| T2 Speech-to-text spike | done | 2026-11-07: engine chosen. Whisper-Medium on the 16 GB Pi's Hailo-10H (HailoRT 5.5.0 built from source). 32 word errors of 108 against 34 to 39 for the other Hailo models and 37 for CPU. See D2, D13, `notes/STT_SPIKE.md` and `notes/HAILO_STT_RESEARCH.md` |
| T3 Speech-to-text server on the Hailo-10H | done | 2026-11-07: C++ server in `scripts/stt/` (cpp-httplib v0.65.0, Whisper-Medium) runs as systemd service `jarvis-stt` on the 16 GB Pi, port 8082, enabled at boot. `scripts/stt/acceptance.sh` passed 25 of 25 from the 8 GB Pi before and after a reboot. DKMS installed for `hailo1x_pci` 5.5.0. Record: `docs/TWO_PI_STT_DEPLOYMENT.md` |
| T4 Speech-to-text client | done | Wiremock-tested, 223 tests passing, committed in `ab3d18b`. Not yet run against the real server from Jarvis (T6 does that) |
| T5 Capture | done | Accepted 2026-11-06 after 1 review round: arecord child process, 15 s byte cap plus a 16 s wall-clock deadline, 0.3 s minimum, kill-on-drop. 9 tests. Committed as `cb21752`. Not yet run against the real Pi mic (T6 acceptance) |
| B1 Long answers fail speech | done | Moved into [real-time-speech](real-time-speech.md) R1 and R3 on 2026-11-06 |
| T6 `/listen` command | in review | Implemented, uncommitted. Shared record-and-transcribe lives in `features/listening/listener.rs` (`Listener`); the CLI keeps the input race and display (`interfaces/cli/repl/listen.rs`). `AssistantState` (`features/state.rs`) is reported through `AssistantPresenter::state` for typed and spoken prompts. 251 tests pass (223 before). Not yet run on the Pi: needs the three `JARVIS_STT_*`/`JARVIS_CAPTURE_DEVICE` settings in the 8 GB Pi's `.env` (Darren's OK) and Darren speaking a question |

**Resume here:** T3 and T4 closed on 2026-11-07. Next is **T6**, the `/listen` command, which wires T5 capture and the T4 client into the CLI. When T6 is accepted, set `JARVIS_STT_BASE_URL=http://jarvis-ai:8082/v1`, `JARVIS_STT_MODEL=whisper-medium` (the server ignores it) and `JARVIS_CAPTURE_DEVICE` (card 3, `plughw:CARD=Device_1`, per D9) in the 8 GB Pi's `.env`; that needs Darren's OK. Still open: `/memory status` shows the embedder ready (T1). Cleanup still open (D7): delete the spike clips in `/tmp/stt-spike` on the 8 GB Pi, in `/tmp` on the 16 GB Pi, and in the session scratchpad. `/tmp/acceptance*.out` on the 8 GB Pi and `~/jarvis-stt/term-test.out` on the 16 GB Pi hold transcripts and belong in the same cleanup. For Darren to decide: `NetworkManager-wait-online.service` fails at boot (60 s timeout) and delays every service ordered after `network-online.target` by about a minute. It is outside this spec, and it is unknown whether it predates the Hailo work. Read `notes/HANDOFF_2026-11-07.md` first.

## Goal

Darren types `/listen`, speaks, presses Enter, and Jarvis answers the transcript exactly as if it had been typed. This is the smallest end-to-end voice path that can be verified on the Pi. No always-on listening and no cloud transcription.

## Decisions

| # | Decision |
| --- | --- |
| D1 | Thin vertical slice; the terminal stays the interface. |
| D2 | **Outcome of the T2 spike (2026-11-07, Darren).** Speech-to-text runs on the 16 GB Pi's Hailo-10H (AI HAT+ 2) with Whisper-Medium, not on the CPU. Darren chose accuracy over speed. Over 7 clips (108 words) Medium made 32 word errors, Small 34, CPU whisper.cpp (small.en, prompt, `--audio-ctx 768`) 37 and Base 39. On the 43 words of short clips: Medium 11, Small 16, CPU 9. Medium takes 1.3 to 3.6 s on short clips and 11.4 s on a 15 s clip. Hailo does not slow Gemma, Gemma does not slow Hailo, and host CPU use is under 3 ms per request. Evidence: `notes/STT_SPIKE.md` and `notes/HAILO_STT_RESEARCH.md`. |
| D3 | Trigger: `/listen` starts recording; Enter stops it. Hard cap of 15 s. No new terminal dependency. |
| D4 | Speech-to-text runs on the 16 GB Pi as an HTTP server (AGENTS.md: inference never runs in-process). Jarvis is a client in `src/clients/` behind the `local_endpoint` guard. The model runs on the Hailo-10H, so it does not compete with Gemma for CPU. |
| D5 | The terminal prints states only: `Idle`, `Listening`, `Transcribing`, `Thinking`, `Speaking`. Audio level for the animation is deferred to the animation spec. |
| D6 | The transcript is printed and sent straight down the normal request path. Empty or unusable audio produces a "didn't catch that" notice and no model request. |
| D7 | Audio is never stored. It stays in memory, goes only to the 16 GB Pi over the LAN, and is dropped after transcription. The transcript is archived like a typed turn, so there is no schema change. Spike clips are throwaway files outside `data/`. |
| D8 | Capture runs `arecord` as a child process (16 kHz, mono, S16_LE WAV). There is no audio crate. `cpal` (already in `Cargo.lock` via rodio) is the fallback if `arecord` proves limiting. |
| D9 | The microphone is the USB PnP mic, ALSA card 3 (`Device_1`), selected by a new `JARVIS_CAPTURE_DEVICE` setting. `JARVIS_AUDIO_DEVICE` stays the Jieli speaker. |
| D10 | Ops on either Pi is done by agents over SSH. Read-only checks need no approval; **every change needs Darren's OK first**. |
| D11 | Sub-agents (Sonnet) implement tickets with explicit, non-overlapping file ownership. Claude reviews and explains how each piece works. Darren reviews and commits. At the end Claude walks Darren through whether the spec was implemented correctly. |
| D12 | **Reuse before writing** (library survey, 2026-11-06, checked against locked rig-core 0.42.0 `src/providers/openai/transcription.rs`). T4 is unchanged by the engine choice. It uses Rig's `TranscriptionModel` through the same `openai::CompletionsClient` recipe as `local_llm.rs`, which POSTs multipart to `{base}/audio/transcriptions` and reads `{"text": ...}`. There is no custom HTTP client and no new crate or feature, since Rig's reqwest already has `multipart`. The WAV wrapper uses `hound`, which is already a dependency. Capture stays `arecord`. Config stays hand-written (`envy`/`figment` would save under 15 lines). The server is our own C++ wrapper around HailoRT's `Speech2Text` using cpp-httplib (T3), not whisper-server. It ignores `prompt` and `model`. |
| D13 | **HailoRT 5.5.0 built by hand** (2026-11-07). Runtime, driver and firmware are built from Hailo's public GitHub source and installed by hand on the 16 GB Pi. The apt 5.1.1 packages are removed and pinned to -1 in `/etc/apt/preferences.d/hailo-block`. Why: 5.1.1 offers only Whisper-Base, and its HEF dropped speech in clips over 10 s; the 5.4.0 and later HEFs fixed that. The Developer Zone needs a login. The driver is installed without DKMS for now, so a kernel update leaves the HAT without a driver until T3 adds DKMS. |
| D14 | **No prompt, no post-correction for now.** Hailo's `Speech2Text` API (through 5.5.0) has no initial-prompt option, so the "Jarvis" vocabulary prompt is not possible. Medium misheard "Jarvis" as "Davies" and "Her job is". No post-correction is added for now. |

## Limits

Each limit is a named constant with tests at its boundaries (last accepted value, first rejected value).

| Constant (suggested) | Value | Meaning |
| --- | --- | --- |
| `MAX_RECORDING` | 15 s = 480,000 bytes of PCM | Recording stops at the cap even without Enter. |
| `MIN_RECORDING` | 0.3 s = 9,600 bytes of PCM | Shorter audio gives "didn't catch that" and is not sent. |
| `TRANSCRIPTION_TIMEOUT` | 20 s | Wait for one transcription request. Kept after T2: Medium measured 11.4 s on a 15 s clip. The Hailo model works in 10 s windows; Medium handled 12 s and 15 s clips without dropping speech. |
| `TRANSCRIPTION_HEALTH_TIMEOUT` | 3 s | Matches the other clients. |
| Transcript length | 2,048 bytes | The same cap as typed input (`MAX_INPUT_BYTES` in `interfaces/cli/input.rs`). |

A timeout means Jarvis stopped waiting. It does not prove the server stopped work.

## Environment facts (verified 2026-11-06)

- **8 GB Pi** `darrenrasp@jarvis`: checkout `/home/darrenrasp/jarvis`, Debian 13 aarch64. Wayland (labwc) under lightdm. 5" DSI touchscreen at 800×480. Chromium installed. `arecord` present, `sqlite3` absent. Capture devices: card 2 Jieli (speaker with a mic input) and card 3 USB PnP mic.
- **Gemma (the brain)**, as Darren runs it: `llama-server -hf unsloth/gemma-4-E2B-it-qat-GGUF:UD-Q4_K_XL --spec-type draft-mtp --spec-draft-n-max 3 -t 4 -c 2048 -b 512 -ub 128 --jinja --host 0.0.0.0 --port 8080 --temp 0 --reasoning off --swa-full`. `--swa-full` (added 2026-11-07, R7 of real-time speech) keeps the whole sliding-window cache so the conversation prefix can be reused between turns. `-t 4` uses all 4 cores, so speech-to-text competes for CPU (T2 measures this). `-c 2048` is a small context shared by history, recall and the transcript.
- **16 GB Pi** `raspdarren-ai@jarvis-ai`: reachable only through the 8 GB Pi (`ssh -J darrenrasp@jarvis raspdarren-ai@jarvis-ai`). The credentials are in the Mac's `.env`.
- **16 GB Pi Hailo state (2026-11-07):**
  - HailoRT 5.5.0 built from source and installed in `/usr`; the manifest is `~/hailo-src/install_manifest-5.5.0.txt`.
  - Driver: `/lib/modules/6.18.50+rpt-rpi-2712/updates/dkms/hailo1x_pci.ko.xz`.
  - Firmware: `/lib/firmware/hailo/hailo10h/`.
  - Device `/dev/h1x-0`, with the udev rule in `/etc/udev/rules.d/51-hailo-udev.rules` (`SUBSYSTEM=="hailo1x", MODE="0666"`).
  - apt pin: `/etc/apt/preferences.d/hailo-block` (the 5.1.1 packages are removed and pinned to -1).
  - HEF: `~/hailo-models/Whisper-Medium.hef`, sha256 `64423764f8138995eb51ef64478ead2ddebeffdf3eec50fe68b8abca0f1613de`.
  - Backups: `~/hailo-backup/`.
  - Rollback: see the bold **Rollback** line just before "Phase 4" in `notes/HAILO_STT_RESEARCH.md` (about line 549).
  - Driver installed through DKMS (2026-11-07), so a new kernel triggers a rebuild. A real kernel upgrade has not been tested yet; check `dkms status` and `/dev/h1x-0` after the first one. The apt pin stays.
  - Service `jarvis-stt`: port 8082, unit at `/etc/systemd/system/jarvis-stt.service`, source in `scripts/stt/`.

## Tickets

Each ticket lists the files it owns. An agent touches nothing else without asking. Code tickets finish with the full verification list in AGENTS.md.

### T0: 16 GB Pi services (ops)
- Diagnose read-only why Gemma and BGE are down (`systemctl`, `journalctl`, shell history, running processes) and why the LAN ping is slow (Wi-Fi power save?).
- With Darren's OK: create systemd units for Gemma (`:8080`) and BGE (`:8081`, `--parallel 1 --alias bge-small-en-v1.5`) that restart on failure and on boot.
- Owns: `docs/TWO_PI_BGE_DEPLOYMENT.md` (record the units actually installed).
- Done when: both `/health` endpoints return 200 from the 8 GB Pi, and still do after a service restart.

### T1: 8 GB Pi cleanup and acceptance (ops)
- With Darren's OK, in order:
  1. `sudo apt install sqlite3`
  2. Back up to `data/jarvis-pre-bge.sqlite3` and verify the backup opens
  3. Remove `JARVIS_EMBEDDING_MODEL_DIR` and `JARVIS_MEMORY_INDEX_DIR` from the Pi's `.env`
  4. Delete `data/memory-index/`
- Then Darren runs the acceptance check by ear: speech start/stop, recall, `/forget`, `/memory status` showing the embedder ready.
- Owns: `docs/MEMORY_STORAGE_OPERATIONS.md` (only if a documented step proved wrong).
- Needs T0.

### T2: speech-to-text spike (throwaway)
- Record about 5 clips with Darren through card 3 (and card 2 for comparison), outside `data/`.
- For each engine: build it on the 16 GB Pi, measure transcription latency at idle and while Gemma is generating, and Darren judges accuracy.
- Start with whisper.cpp `whisper-server --request-path /v1 --inference-path /audio/transcriptions`, so it serves the OpenAI path Rig posts to. Confirm with one curl that it ignores Rig's `model` form field.
- Output: `notes/STT_SPIKE.md` with the table, the chosen engine, its HTTP API (endpoint, request and response format), and any timeout revision. No code in `src/`.
- Needs T0. Darren in the loop.

### T3: Speech-to-text server on the Hailo-10H (C++ + ops)
- A C++ server built on the 16 GB Pi against the installed HailoRT 5.5.0, plus cpp-httplib (header-only). Pin an exact cpp-httplib version; that dependency needs Darren's approval of the version.
- It loads Whisper-Medium once at start (about 6 s). Health is served on both `/health` and `/v1/health` (the T4 client probes `/health` at the root); both return 200 only after the model has loaded.
- The model file path is a setting in the systemd unit (an environment variable or argument), not compiled in, so switching between Whisper-Medium and Whisper-Small (`~/hailo-models/Whisper-Small.hef`) needs only a config change and a restart. Darren may compare them in daily use: Small is about 2.5 times faster, Medium more accurate on short commands.
- `POST /v1/audio/transcriptions` takes multipart with a required `file`: WAV, 16 kHz, mono, S16_LE. It accepts and ignores `model`, `prompt` and `temperature`, and accepts `language` only as `en`. It returns `{"text": "..."}`.
- It joins the segment texts. It drops segments that start at or after the audio's end (Medium invented "you" at 14.88 to 16.88 s on a 15 s clip). It turns output made only of "..." or blank-audio markers into empty text, so Jarvis's "didn't catch that" path (D6) triggers.
- It rejects bad input with 4xx: missing file, wrong WAV format, audio over 15 s plus a small margin, a body over a byte limit.
- It serves one request at a time.
- It has a request read timeout. whisper-server hung on a POST with no body; test that this server does not.
- It binds LAN-only, runs as a non-root user, and is a systemd unit `jarvis-stt` (restart on failure, enabled at boot, ordered after the driver loads).
- DKMS for `hailo1x_pci` 5.5.0, built from the drivers repo's `dkms.conf.in` (`make clean` first), replacing the hand-copied module. Verify with `dkms status`, and confirm that a reboot still brings up `/dev/h1x-0`.
- Owns: `docs/TWO_PI_STT_DEPLOYMENT.md`. Source: `scripts/stt/` (settled with Darren, 2026-11-07).
- Done when:
  - curl from the 8 GB Pi returns the transcript for the 6 clips and Darren's real 15 s clip (`clip6-long`).
  - A reboot of the 16 GB Pi brings `jarvis-stt` back with `/v1/health` 200.
  - Gemma and BGE are unaffected.
- Needs Darren's OK for every Pi change (D10).
- Done 2026-11-07; record in `docs/TWO_PI_STT_DEPLOYMENT.md`.

### T4: speech-to-text client (Rust)
- A thin `src/clients/speech_to_text.rs` built on Rig's `TranscriptionModel` (D12), reusing the `LocalLlm::new` client recipe: client-level timeout, `retry(never())`, `no_proxy`, `LocalEndpointResolver`, no redirects. Wrap PCM as WAV with `hound`, trim the returned text, and map `TranscriptionError` into a thiserror enum. Rig has no per-request timeout, so the timeout lives on the client.
- `src/config/speech_to_text.rs`: base URL, model or alias, `JARVIS_CAPTURE_DEVICE`. Paths are resolved to absolute here.
- Tests: `tests/unit/clients/speech_to_text_tests.rs` (Wiremock: wire format, timeout, non-2xx, oversize transcript) and config tests.
- Owns those files plus the `mod` lines in `src/clients/mod.rs` and `src/config/mod.rs`.
- Unblocked: the wire format is settled and unchanged (D12). It does not depend on T3 and can run in parallel with it.

### T5: capture (Rust)
- `src/features/listening/`: start `arecord` with the configured device (16 kHz mono S16_LE), stop on request or at `MAX_RECORDING`, return in-memory audio, reject below `MIN_RECORDING`. Kill and reap the child on drop, so it is safe inside `select!`. Blocking work goes in `spawn_blocking` or a tokio process.
- Tests in `tests/unit/features/listening/`, covering the boundaries at 9,599/9,600 and 480,000/480,001 bytes. Use a fake recorder script instead of a real mic.
- Owns `src/features/listening/**`, its tests, and the `mod` line in `src/features/mod.rs`.
- Independent; can start now.

### T6: `/listen` command (Rust)
- `Command::Listen` with a `parse` arm and a `COMMANDS` row in `features/control/command.rs`.
- The CLI handles it because it controls the interface (Enter stops it). It prints the states (D5), then the transcript, then feeds the transcript through the same path as typed input, including the 2,048-byte cap.
- New prompts and `/stop` still behave as they do today.
- Owns: `features/control/command.rs` and its tests, `interfaces/cli/repl/**` and its tests, and the wiring in `src/app.rs`.
- Pi acceptance: Darren says a question and hears the answer.
- Needs T4, T5.

## Out of scope (next specs)

- **Jarvis animation:** Darren wants actual 2D art and animation: an on-screen Jarvis character he can see speaking, not only an abstract audio-reactive shape (requirement of 2026-11-07). It is a local web page served by Jarvis, full-screen in Chromium on the 5" screen, and later the remote-control page. Needs approval for a web-server dependency (e.g. `axum`), LAN-only. Adds audio level to the state stream.
  - **An original 2D Jarvis character.** The films' JARVIS has no face, so this is Darren's own design. It must not copy the films' interface or likeness.
  - **Inputs it needs from Jarvis:**
    - the assistant state stream from T6 (`Idle`, `Listening`, `Transcribing`, `Thinking`, `Speaking`), which picks the pose or animation
    - the speech audio level, sampled about 30 times a second while speaking, so the mouth moves with loudness
    - later, optionally, phoneme or viseme timing for real lip-sync. Whether Piper can provide phoneme timings is NOT verified; check it when that spec starts.
    - a separate emotion field for expression or colour (Darren's example: red for anger), not mixed into the state enum
  - **"Speaking" must follow actual audio playback,** start and end, not text streaming, so the mouth stays in sync.
  - **Rendering:** a local web page served by Jarvis, full-screen in Chromium on the 8 GB Pi's 5" screen. A web-server crate needs Darren's approval.
  - **Art:** frame sets per state and mouth shape, or a rigged character (for example Rive, which has state machines and a web runtime: not evaluated yet). The art itself comes from Darren or an artist.
- Calendar read (8A), hold-to-talk or a GPIO button, wake words.
