# 02: Spike: VAD on Darren's mic

Status: done
Stories: US5, US6
Blocked by: None
Assignee: Darren
Owns: notes/VAD_SPIKE.md, notes/vad-spike/**

**What to build:** Compare Rust speech-detection (VAD) options on real recordings from Darren's USB PnP mic (ALSA card 3), and recommend one crate. Candidates include a Silero VAD wrapper (ONNX runtime) and the WebRTC VAD; the agent surveys others and checks that each builds for aarch64. Darren speaks the clips: short questions, long sentences, a pause in the middle of a sentence, background noise (TV or fan), and silence. The agent records them with `arecord` as Jarvis does (16 kHz mono S16_LE). Throwaway code stays in `notes/vad-spike/`, with no code in `src/` and no change to `Cargo.toml`. The chosen crate needs Darren's approval before ticket 06 uses it. Clips are throwaway files outside `data/`; delete them when the spike closes (push-to-talk D7).

## Acceptance criteria
- [x] At least two options are compared on the same clips, each with its version and licence.
- [x] For each option: false cut-offs (it ended in a mid-sentence pause), missed ends (it never ended), and the end-of-speech delay in milliseconds after Darren stops.
- [x] CPU use per second of audio on the 8 GB Pi 5, and the extra memory or runtime files an option needs.
- [x] The build works on aarch64 Linux, or the failure is recorded.
- [x] A recommended crate and the silence length that ends a recording, with the trade-offs, in `notes/VAD_SPIKE.md`. Darren's approval of the crate is recorded in Comments.
- [x] The clips are deleted from the Pis and the scratchpad when done.

## Comments

2026-11-11 (Claude): desk half done by an agent; the real clips need Darren's voice. Nine candidates surveyed in `notes/VAD_SPIKE.md`. A scratch harness in `notes/vad-spike/` builds and runs on the Mac with three engines: `earshot =1.2.2` (pure Rust, weights embedded), Silero v5 through `voice_activity_detector =0.2.1` (needs `ort =2.0.0-rc.10` and a prebuilt ONNX Runtime downloaded at build time) and `webrtc-vad =0.4.0` (needs a C compiler). Sanity-checked on 8 synthetic `say` clips only: the silence rules fire as designed, and earshot and WebRTC "quality" mode flagged the first frame or two of pure noise as speech, so ticket 06 needs a minimum speech length. Provisional, not based on Darren's voice or the Pi: try earshot first with a silence rule of about 1000 ms, and keep Silero as the accuracy reference. Nothing about the Pi is verified (aarch64 build, CPU, memory). Next: Darren records the nine clips with the commands in section 4 of `notes/VAD_SPIKE.md`. No acceptance criterion is ticked yet.

2026-11-14 (Claude): Darren recorded the nine clips; scored by Claude on the Mac (six engine modes) and on the 8 GB Pi (earshot and WebRTC; decisions identical to the Mac's). Full results in `notes/VAD_SPIKE.md` section 7.
- **Recommend `earshot =1.2.2`.** It matches Silero on every clip (segments within about 60 ms, same verdicts, no false speech in room silence), builds on the Pi with `--locked` in 4.9 s, and costs 0.96 ms of one core per second of audio. WebRTC hears the start click and blips in room silence in every mode.
- **Silence length is open.** Darren's pauses measured 1568 ms and 3040 ms, so 700, 1000 and 1500 ms all cut both pause clips; short questions and long sentences were clean at all three.
- **noise1 never ends** at 1000 or 1500 ms on any engine. Needs Darren's note on what the noise was.
- **Not ticked:** CPU and memory per option (Silero was not built on the Pi: ONNX Runtime download not approved, and it would not change the pick; memory is an upper bound of 11 MiB for the process), Darren's approval of the crate and the silence length, and deleting the clips (kept until he approves, with `~/vad-spike` on the Pi).

2026-11-14 (Claude, after Darren's reply): **Darren approved `earshot =1.2.2` and a silence length of 1000 ms.** He is not sure what the noise in `noise1` was; it does not change the pick, because the tap and the 15 s recording cap end a recording when the room has voices in it. CPU and memory are recorded for earshot and WebRTC on the Pi; Silero was not built there (it needs an ONNX Runtime download and matched earshot on every clip), which is the recorded reason for that gap. Clips deleted by Claude: `/tmp/vad-spike` and `~/vad-spike` on the 8 GB Pi and the Mac scratchpad copy. `notes/vad-spike/` (the harness and the synthetic clips, untracked) stays as the record.
