# 13: A short sound on every state change

Status: needs-info
Stories: US1, US2, US3
Blocked by: fast-memory 08 (order only, Darren's choice); the sound preview below
Assignee: Claude
Owns: to be listed once the preview is chosen: a new `src/features/speech/cues.rs` with its tests, `src/features/speech/playback.rs` and `output.rs` (playing a cue through the open output), `src/config/` (volume and off switch), and the place in `src/interfaces/` where the state changes

**Why:** On 2026-11-15 Darren asked for sound effects on Jarvis's state animations, in the style of the Great Sage / Raphael sounds from "That Time I Got Reincarnated as a Slime" and of Iron Man's JARVIS. A sound tells him what Jarvis is doing when he is not looking at the screen, and a turn that ends with nothing spoken is silent today.

**Settled with Darren by grilling, 2026-11-15 (spec D16):**

- **A cue on every state change, and one for errors** (nothing heard, speech-to-text failed, the model timed out): six cues, one each for listening, transcribing, thinking, speaking, idle and error. The idle cue is the quietest.

- **WAV files from a folder that is not in git** (changed 2026-11-15, replacing "generated in code"). Three generated sets did not sound right to Darren: he wants the sounds of his reference clip itself ("Jarvis startup welcome sound", `https://www.youtube.com/watch?v=oY2nGQfuuuQ`), not an imitation. The repository is public and the clip is someone else's audio, so Jarvis loads one WAV file per cue from `data/sounds/` (`data/` is ignored by git) and the files are copied to the Pi by hand. A missing file means that cue is silent, with one startup warning. `rodio` already has its `wav` feature.
- **The microphone must not record the listening cue.** It stays under about 150 ms and recording starts when it ends. Measure on the Pi whether the first word gets clipped before trying anything more complicated.
- **The voice starts over the tail of the speaking ping,** about 120 ms in, so the ping adds almost nothing to the wait.
- **The idle cue is skipped when another state follows within a moment** (a tap that interrupts goes straight to the listening cue).
- **Config:** a volume and an off switch.
- **Preview first.** `notes/sound-spike/out_ref/` holds a first cut of the six cues, sliced from the reference clip (listening 8.24 to 8.40 s, transcribing 8.40 to 8.63 s, thinking 8.63 to 9.05 s, speaking 8.98 to 9.20 s, idle 3.67 to 3.87 s, error 3.81 to 4.02 s). Which slice suits which state is Claude's guess from measurements; Darren has not confirmed it. The Pi speaker is the final check, since a small speaker loses low notes.
- **No new crate:** Jarvis plays the cues through the output stream it already keeps open (`AudioPlayer`'s mixer).

**Code facts:**
- `playback.rs` queues a 700 ms near-silent wake noise before each answer (`WAKE_NOISE_DURATION`). Check whether the speaking ping can play inside that window instead of adding to it.
- The cues are played by Jarvis, not by `jarvis-orb`: Jarvis owns the audio output and knows the real state (spec D7, D9).

**Open:** whether each slice is the right one for its state; the exact files in `Owns:`.

## Acceptance criteria
- [ ] Darren has picked each cue from the preview, and his choices are recorded here.
- [ ] Cue files are read once at startup from the configured folder; a missing or unreadable file makes that cue silent with one warning and never stops Jarvis. Tested with present, missing and broken files.
- [ ] The cue files are not in git.
- [ ] Each state change plays its cue once; a turn that ends with nothing spoken plays the error cue.
- [ ] The listening cue is not in the clip sent to speech-to-text (a test on the recording start, and a check of a real clip on the Pi).
- [ ] The idle cue is skipped when a tap interrupts.
- [ ] The off switch plays nothing and changes no timing; the volume boundary values are tested.
- [ ] No new crate. `scripts/verify.sh` passes.
- [ ] Darren has heard all six on the Pi speaker in a real conversation.

## Comments
