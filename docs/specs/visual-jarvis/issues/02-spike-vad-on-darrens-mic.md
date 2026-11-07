# 02: Spike: VAD on Darren's mic

Status: ready-for-human
Stories: US5, US6
Blocked by: None
Assignee: unassigned
Owns: notes/VAD_SPIKE.md, notes/vad-spike/**

**What to build:** Compare Rust speech-detection (VAD) options on real recordings from Darren's USB PnP mic (ALSA card 3), and recommend one crate. Candidates include a Silero VAD wrapper (ONNX runtime) and the WebRTC VAD; the agent surveys others and checks that each builds for aarch64. Darren speaks the clips: short questions, long sentences, a pause in the middle of a sentence, background noise (TV or fan), and silence. The agent records them with `arecord` as Jarvis does (16 kHz mono S16_LE). Throwaway code stays in `notes/vad-spike/`, with no code in `src/` and no change to `Cargo.toml`. The chosen crate needs Darren's approval before ticket 06 uses it. Clips are throwaway files outside `data/`; delete them when the spike closes (push-to-talk D7).

## Acceptance criteria
- [ ] At least two options are compared on the same clips, each with its version and licence.
- [ ] For each option: false cut-offs (it ended in a mid-sentence pause), missed ends (it never ended), and the end-of-speech delay in milliseconds after Darren stops.
- [ ] CPU use per second of audio on the 8 GB Pi 5, and the extra memory or runtime files an option needs.
- [ ] The build works on aarch64 Linux, or the failure is recorded.
- [ ] A recommended crate and the silence length that ends a recording, with the trade-offs, in `notes/VAD_SPIKE.md`. Darren's approval of the crate is recorded in Comments.
- [ ] The clips are deleted from the Pis and the scratchpad when done.

## Comments
