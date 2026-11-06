# Spec: push-to-talk (roadmap 15A, thin slice)

Settled with Darren in a grilling session on 2026-11-06. This file is the source of truth for the work in progress. **Update the Status section at the end of every session** so the next session can resume from it.

## Status

| Ticket | State | Notes |
| --- | --- | --- |
| T0 16 GB Pi services | done | Installed 2026-11-06: `jarvis-gemma` and `jarvis-embeddings` systemd units, enabled; `/health` 200 from the 8 GB Pi after install and after restart (re-checked by Claude); BGE alias served. Wi-Fi power save off on both Pis. Remaining: Darren reboots the 16 GB Pi once to prove boot start |
| T1 8 GB Pi cleanup + acceptance | in review | Cleanup done and re-checked 2026-11-06. By ear: speech plays, `/stop` works; a long answer failed and speech then stayed off until restart (now real-time-speech R1). Recall and `/forget` not yet checked |
| T2 Speech-to-text spike | not started | Unblocked by T0. Needs Darren to speak the clips |
| T3 Speech-to-text server | not started | Blocked on T2 (engine choice) |
| T4 Speech-to-text client | not started | Blocked on T2 (wire format) |
| T5 Capture | done | Accepted 2026-11-06 after 1 review round: arecord child process, 15 s byte cap plus a 16 s wall-clock deadline, 0.3 s minimum, kill-on-drop. 9 tests; 107 unit tests and clippy pass. Not yet run against the real Pi mic (T6 acceptance). Awaiting Darren's commit |
| B1 Long answers fail speech | done | Moved into [real-time-speech](real-time-speech.md) R1 and R3 on 2026-11-06 |
| T6 `/listen` command | not started | Blocked on T4, T5 |

**Resume here:** paused at T2 while [real-time-speech](real-time-speech.md) runs (decided 2026-11-06). Still open: Darren reboots the 16 GB Pi, checks recall and `/forget` (T1), and commits T0/T5.

## Goal

Darren types `/listen`, speaks, presses Enter, and Jarvis answers the transcript exactly as if it had been typed. This is the smallest end-to-end voice path that can be verified on the Pi. No always-on listening and no cloud transcription.

## Decisions

| # | Decision |
| --- | --- |
| D1 | Thin vertical slice; the terminal stays the interface. |
| D2 | A measurement spike (T2) picks the speech-to-text engine from real Pi numbers: whisper.cpp first, then Moonshine and `parakeet-rs` if they build for aarch64. |
| D3 | Trigger: `/listen` starts recording; Enter stops it. Hard cap of 15 s. No new terminal dependency. |
| D4 | Speech-to-text runs on the 16 GB Pi as an HTTP server (AGENTS.md: inference never runs in-process). Jarvis is a client in `src/clients/` behind the `local_endpoint` guard. |
| D5 | The terminal prints states only: `Idle`, `Listening`, `Transcribing`, `Thinking`, `Speaking`. Audio level for the animation is deferred to the animation spec. |
| D6 | The transcript is printed and sent straight down the normal request path. Empty or unusable audio produces a "didn't catch that" notice and no model request. |
| D7 | Audio is never stored. It stays in memory, goes only to the 16 GB Pi over the LAN, and is dropped after transcription. The transcript is archived like a typed turn, so there is no schema change. Spike clips are throwaway files outside `data/`. |
| D8 | Capture runs `arecord` as a child process (16 kHz, mono, S16_LE WAV). There is no audio crate. `cpal` (already in `Cargo.lock` via rodio) is the fallback if `arecord` proves limiting. |
| D9 | The microphone is the USB PnP mic, ALSA card 3 (`Device_1`), selected by a new `JARVIS_CAPTURE_DEVICE` setting. `JARVIS_AUDIO_DEVICE` stays the Jieli speaker. |
| D10 | Ops on either Pi is done by agents over SSH. Read-only checks need no approval; **every change needs Darren's OK first**. |
| D11 | Sub-agents (Sonnet) implement tickets with explicit, non-overlapping file ownership. Claude reviews and explains how each piece works. Darren reviews and commits. At the end Claude walks Darren through whether the spec was implemented correctly. |
| D12 | **Reuse before writing** (library survey, 2026-11-06, checked against locked rig-core 0.42.0 `src/providers/openai/transcription.rs`). T4 uses Rig's `TranscriptionModel` through the same `openai::CompletionsClient` recipe as `local_llm.rs`, which POSTs multipart to `{base}/audio/transcriptions`. There is no custom HTTP client and no new crate or feature, since Rig's reqwest already has `multipart`. The WAV wrapper uses `hound`, which is already a dependency. Capture stays `arecord`. Config stays hand-written (`envy`/`figment` would save under 15 lines). |

## Limits

Each limit is a named constant with tests at its boundaries (last accepted value, first rejected value).

| Constant (suggested) | Value | Meaning |
| --- | --- | --- |
| `MAX_RECORDING` | 15 s = 480,000 bytes of PCM | Recording stops at the cap even without Enter. |
| `MIN_RECORDING` | 0.3 s = 9,600 bytes of PCM | Shorter audio gives "didn't catch that" and is not sent. |
| `TRANSCRIPTION_TIMEOUT` | 20 s | Wait for one transcription request. T2 may revise this from measurements. |
| `TRANSCRIPTION_HEALTH_TIMEOUT` | 3 s | Matches the other clients. |
| Transcript length | 2,048 bytes | The same cap as typed input (`MAX_INPUT_BYTES` in `interfaces/cli/input.rs`). |

A timeout means Jarvis stopped waiting. It does not prove the server stopped work.

## Environment facts (verified 2026-11-06)

- **8 GB Pi** `darrenrasp@jarvis`: checkout `/home/darrenrasp/jarvis`, Debian 13 aarch64. Wayland (labwc) under lightdm. 5" DSI touchscreen at 800×480. Chromium installed. `arecord` present, `sqlite3` absent. Capture devices: card 2 Jieli (speaker with a mic input) and card 3 USB PnP mic.
- **Gemma (the brain)**, as Darren runs it: `llama-server -hf unsloth/gemma-4-E2B-it-qat-GGUF:UD-Q4_K_XL --spec-type draft-mtp --spec-draft-n-max 3 -t 4 -c 2048 -b 512 -ub 128 --jinja --host 0.0.0.0 --port 8080 --temp 0 --reasoning off --swa-full`. `--swa-full` (added 2026-11-07, R7 of real-time speech) keeps the whole sliding-window cache so the conversation prefix can be reused between turns. `-t 4` uses all 4 cores, so speech-to-text competes for CPU (T2 measures this). `-c 2048` is a small context shared by history, recall and the transcript.
- **16 GB Pi** `raspdarren-ai@jarvis-ai`: reachable only through the 8 GB Pi (`ssh -J darrenrasp@jarvis raspdarren-ai@jarvis-ai`). The credentials are in the Mac's `.env`.

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

### T3: speech-to-text server (ops + docs)
- With Darren's OK: install the chosen engine as a systemd service on the 16 GB Pi, LAN-only.
- Owns: `docs/TWO_PI_STT_DEPLOYMENT.md`.
- Needs T2.

### T4: speech-to-text client (Rust)
- A thin `src/clients/speech_to_text.rs` built on Rig's `TranscriptionModel` (D12), reusing the `LocalLlm::new` client recipe: client-level timeout, `retry(never())`, `no_proxy`, `LocalEndpointResolver`, no redirects. Wrap PCM as WAV with `hound`, trim the returned text, and map `TranscriptionError` into a thiserror enum. Rig has no per-request timeout, so the timeout lives on the client.
- `src/config/speech_to_text.rs`: base URL, model or alias, `JARVIS_CAPTURE_DEVICE`. Paths are resolved to absolute here.
- Tests: `tests/unit/clients/speech_to_text_tests.rs` (Wiremock: wire format, timeout, non-2xx, oversize transcript) and config tests.
- Owns those files plus the `mod` lines in `src/clients/mod.rs` and `src/config/mod.rs`.
- Needs T2's API.

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

- **Jarvis animation:** an original audio-reactive design (not a copy of the film's interface). It is a local web page served by Jarvis, full-screen in Chromium on the 5" screen, and later the remote-control page. Needs approval for a web-server dependency (e.g. `axum`), LAN-only. Adds audio level to the state stream.
- Calendar read (8A), hold-to-talk or a GPIO button, wake words.
