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
| Local speech output | Supertonic worker + Rodio playback; `/stop`; new prompt cancels speech |
| Long-term memory | Schema v4: chunked memories, FTS5 + sqlite-vec hybrid recall with RRF, lineage-aware forgetting, keyword-only fallback |

## Next: voice MVP

Work through these in order. Each step ends with a commit and a short note of what was verified.

1. **Pi acceptance of the current build.** Back up `data/jarvis.sqlite3`, run the v4 migration on the 8 GB Pi, and check speech start/stop, recall, `/forget`, and `/memory status` on real hardware.
2. **Remote BGE embeddings.** Run [the deployment benchmark](TWO_PI_BGE_DEPLOYMENT.md) on the 16 GB Pi, then follow [the integration guide](BGE_SERVER_INTEGRATION.md). This removes ONNX Runtime from the 8 GB Pi.
3. **Push-to-talk input (15A).** Capture worker → bounded audio → speech-to-text → the normal request path. Evaluate Whisper.cpp, `parakeet-rs` and Moonshine on the Pi before choosing. No always-listening and no cloud transcription.
4. **Second tool: read one calendar (8A).** Add it to `ALLOWED_TOOLS`. The first tool that *writes* (8B, create an event) also introduces per-tool permission levels with explicit, content-bound user approval.

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

## Open decisions

| Decision | Settle before | Default until then |
| --- | --- | --- |
| Archive retention and deletion | Long-running use | Keep everything; `/reset` never deletes |
| ASR engine | 15A | None chosen; measure on the Pi |
| Calendar provider | 8A | One provider adapter |
| Approval semantics for writes | 8B | Content-bound approval, no blind retry |
