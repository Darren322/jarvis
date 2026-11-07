# Jarvis roadmap

Jarvis is a local-first voice assistant for a two-Pi cluster:

| Host | Runs |
| --- | --- |
| Raspberry Pi 5, 8 GB (`jarvis`) | Jarvis (Rust/Tokio), local TTS worker, SQLite archive and memory |
| Raspberry Pi 5, 16 GB (`jarvis-ai.local`) | llama.cpp Gemma chat server; planned BGE embedding server |

This file is the short, current plan. The full design history (revision 22 and earlier) lives in `notes/archive/` and is not a source of rules.

## Principles that stay fixed

- **Local by default.** Nothing leaves the LAN without an explicit opt-in that covers the whole context sent, including history and tool results.
- **Model output is untrusted.** Model text, tool arguments, recalled memory and speech recognition never grant permission. Authorization comes from trusted caller context.
- **Rig owns the model loop.** Jarvis owns policy (allowed tools, turn limits, budgets), privacy and presentation; it does not reimplement tool dispatch.
- **One durable store.** SQLite is the single source of truth for the archive and memory. Derived indexes live inside it so writes and deletes are transactional.
- **Timeouts mean "stopped waiting".** A timeout never proves remote or native work stopped.
- **Add things when a concrete need exists.** No speculative layers, wrappers or dependencies.

## Done

| Capability | Notes |
| --- | --- |
| Bounded Rig run with policy | Two model turns max; allowed-tools list (`tools::ALLOWED_TOOLS`); per-tool typed argument checks |
| Run observation | Per-stage timing and usage, explicitly "unavailable" when not observed |
| Terminal conversation + archive | Streaming text; bounded native history; every turn archived in SQLite |
| Local speech output | Real-time speech ([spec](specs/real-time-speech.md), closed 2026-11-07): speech streams while Gemma generates, Piper `en_GB-northern_english_male-medium` (Supertonic selectable), per-answer recovery, `/stop`; first audio about 2-3.5 s on short questions on the Pi |
| Prompt-cache reuse | Gemma runs with `--swa-full`; recalled memories go before the last exchange so the older conversation stays cached |
| Long-term memory | Schema v4: chunked memories, FTS5 + sqlite-vec hybrid recall with RRF, lineage-aware forgetting, keyword-only fallback |

## Next: voice MVP

Work through these in order. Each step ends with a commit and a short note of what was verified.

1. **Pi acceptance of the current build.** Speech start/stop, recall, `/memories` and `/forget <id>` were all exercised on the Pi on 2026-11-07. Remaining: `/memory status` showing the embedder ready.
2. **Remote BGE embeddings.** The server runs on the 16 GB Pi and Jarvis uses it (`3287032`). The systemd services and the 8 GB Pi switch were done in push-to-talk T0/T1 (2026-11-06). The reboot test is done: the 16 GB Pi rebooted twice on 2026-11-07 (Hailo installs), and `jarvis-gemma` and `jarvis-embeddings` came back each time; Gemma `/health` returned 200 about 3 minutes after boot. Remaining: measuring Gemma latency while Jarvis re-embeds.
3. **Push-to-talk input (15A).** Next up: T2 (the speech-to-text spike) is done; T3 (Hailo server) and T4 (client) are next. Spec and tickets: [specs/push-to-talk.md](specs/push-to-talk.md). Capture worker → bounded audio → speech-to-text → the normal request path. Engine chosen: Whisper-Medium on the Hailo-10H. No always-listening and no cloud transcription.
4. **Memory quality.** Found during real-time speech (2026-11-07). Jarvis saves every exchange, its own replies included, as a recallable memory. A wrong reply was recalled and copied on similar questions, and each copy was saved again (memory poisoning; fixed once by hand with `/forget`). Recommended first step, to grill before a spec:
   - Make only Darren's side of each exchange recallable; the full exchange stays in the `turns` archive. One change point: `conversation_text` in `src/storage/chunking.rs`, plus a one-off rebuild of existing memories (back up first; memory text is immutable).
   - Skip recalled exchanges that nearly repeat the current question.
   - Add a recall token budget (about 200-300 tokens): today a story question recalls about 1,000 tokens, roughly 17 s of prompt reading on the Pi.

   Also in scope:
   - Find the cause of the first-turn "embedding server is unavailable" warning (the server answers `ok`, so print the error's source chain).
   - Stop the 3-exchange history window from changing the cached prefix every turn.
   - Make a bare `/forget` show its usage instead of going to Gemma.

   Later, only if needed: idle-time fact extraction, which stores model-written facts (untrusted) linked to their turns. Push-to-talk transcripts flow into the same memory, so settle the first step soon after 15A.
5. **Second tool: read one calendar (8A).** Add it to `ALLOWED_TOOLS`. The first tool that *writes* (8B, create an event) also introduces per-tool permission levels with explicit, content-bound user approval.

## Later (named, in rough order)

| Phase | Capability | Depends on |
| --- | --- | --- |
| 4C | Model-aware context budget before every request | Larger recall or cloud use |
| 6 | One supervised background worker (health) | Continuous operation |
| 11A–C | Camera: one clip → one live Tapo C100 stream → observations delivered to Rust | 6 |
| 12A | Facial recognition, offline: enroll Darren only, evaluate matching | 11A, usable imagery |
| 12B | Opt-in household recognition in shadow mode | 11C, 12A |
| 9 | One explicitly permitted cloud request | 4C, privacy opt-in |

**Facial recognition constraints.** Detection, alignment and embeddings run on the 16 GB Pi; evaluate OpenCV YuNet + SFace first. Matching uses an in-memory gallery of a handful of people, so no extra vector service is needed. Enrollment templates and sightings are stored in SQLite, and un-enrolling must delete them transactionally. Always allow "unknown" and "ambiguous". A face match may personalize a greeting. It never authenticates a speaker, unlocks anything, approves a write or reveals private data. Enrollment requires each person's agreement, and face data never leaves the LAN.

## Someday (not planned)

Recorded-clip storage and remote playback (11D–E), sensors and physical actions (10A–B), occupancy ML (13–14), a UI or API (16), multi-agent experiments (9B), and offline improvement proposals (17).

Follow-ups from the speech-to-text spike (2026-11-07):

- **Speech synthesis on the Hailo-10H.** Hailo ships no TTS model that we found; hailo-apps runs Piper on the CPU. It would need a compiled model and audio streamed across the LAN. Idea only.
- **Wake word and speech detection.** Darren wants a "Hey Jarvis" wake word later.
  - A small always-on detector runs on the 8 GB Pi, where the mic is.
  - Speech detection (VAD) ends the recording instead of Enter.
  - Only the command after the wake word goes to Whisper on the HAT, so Whisper misses on the name stop mattering (push-to-talk D14).
  - openWakeWord may ship a pretrained "hey jarvis" model: verify before relying on it. Survey Rust-native detectors too.
  - Detection is local and no audio is stored (push-to-talk D7).
  - It reuses the T3/T4 transcription path; T5 and T6 stay as the push-to-talk path.
  - References: hailo-apps `v2a_demo` (openWakeWord, VAD) and `martincerven/hailo_learn` voice_assistant (Silero VAD, openWakeWord, Qwen on a second HAT).
- **Jarvis animation.** The 2D character idea is superseded by the orb design in `specs/visual-jarvis/spec.md` (an original orb on the 5" touchscreen, driven over a Unix socket).
- **The HAT will be shared with future vision work.** Use a shared VDevice or group ID, and test concurrent use; there is a forum report of a multi-model wedge on 5.3.0.

## Open decisions

| Decision | Settle before | Default until then |
| --- | --- | --- |
| Archive retention and deletion | Long-running use | Keep everything; `/reset` never deletes |
| ASR engine | 15A | Chosen 2026-11-07: Whisper-Medium on the Hailo-10H (push-to-talk D2) |
| Calendar provider | 8A | One provider adapter |
| Approval semantics for writes | 8B | Content-bound approval, no blind retry |
