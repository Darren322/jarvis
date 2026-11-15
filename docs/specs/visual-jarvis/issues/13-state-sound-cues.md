# 13: A short sound on every state change

Status: in-review
Stories: US1, US2, US3
Blocked by: fast-memory 08 (order only, Darren's choice); the sound preview below
Assignee: Claude
Owns: `src/features/speech/cues.rs`, `playback.rs`, `output.rs` and `mod.rs`; `src/config/cues.rs` and `mod.rs`; `src/interfaces/cli/repl/mod.rs` and `speech.rs`; `src/app.rs`; `.env.example`; tests in `tests/unit/features/speech/cues_tests.rs` and `playback_tests.rs`, `tests/unit/config/cues_tests.rs`, `tests/unit/interfaces/cli/repl/repl_tests.rs`

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

**Design (2026-11-15):**
- Cues play through the audio output that speech already owns. `AudioPlayer` offers its mixer on a `watch` channel for as long as it is open; a `CuePlayer` (a cheap clone handle) adds a cue to that mixer. With speech off there is no audio device, so there are no cues.
- At startup `App` loads the files on a blocking thread, prints one warning naming those that did not load, and, when at least one loaded, opens the audio device at once instead of at the first answer, so a cue can play before Jarvis has spoken.
- A task beside the REPL observes `StateFeed` and plays the cue for each change (`run_state_cues`), deciding with the pure `CueSequence`: transcribing, thinking and speaking play their own cue; reaching idle plays the idle cue if Jarvis spoke in that turn and the error cue otherwise.
- The listening cue is different: the REPL plays it before the recorder starts and waits for its length (`SpeechControl::play_listening_cue`, which does nothing by default), so the recording and the silence detector never hear it.
- A tap that interrupts an answer never passes through idle (thinking or speaking goes straight to listening), so no idle cue plays there; no timer is needed.
- The speaking state is reported when the first chunk is queued, which is the start of the 700 ms wake noise, so the speaking cue plays inside that window and adds nothing to the wait.
- Settings: `JARVIS_CUE_DIR` (default `data/sounds`) and `JARVIS_CUE_VOLUME` (0 to 1, default 0.7; 0 is off). A file must be 16-bit mono PCM, 16,000 to 48,000 Hz, at most 2 s.

**Open:** whether each slice is the right one for its state; whether the USB speaker clips a cue that follows silence; a deliberate cancel (`/stop` while listening, a command whose reply is only printed) reaches idle without speech and so plays the error cue.

## Acceptance criteria
- [x] Darren has picked each cue from the preview, and his choices are recorded here.
- [x] Cue files are read once at startup from the configured folder; a missing or unreadable file makes that cue silent with one warning and never stops Jarvis. Tested with present, missing and broken files.
- [x] The cue files are not in git.
- [x] Each state change plays its cue once; a turn that ends with nothing spoken plays the error cue.
- [ ] The listening cue is not in the clip sent to speech-to-text (a test on the recording start, and a check of a real clip on the Pi).
- [x] The idle cue is skipped when a tap interrupts.
- [x] The off switch plays nothing and changes no timing; the volume boundary values are tested.
- [x] No new crate. `scripts/verify.sh` passes.
- [ ] Darren has heard all six on the Pi speaker in a real conversation.

## Comments

2026-11-15 (Claude): Darren played the six slices in `notes/sound-spike/out_ref/` on the Pi speaker (`turn.wav`) and said "the audio is fine". That approves the set; he was not asked about each slice's state one by one. Building started in the main checkout and stopped part-way for a handoff: the working tree does not compile. What exists, what is left and the open risks (above all, the speaker may clip a cue that follows silence) are in `notes/HANDOFF_2026-11-15.md`.

2026-11-15 (Claude), later session: built and in review, uncommitted on `dev`. The coordinator finished the wiring (the listening cue in `repl/mod.rs`, startup loading and the cue task in `app.rs`, the two settings in `.env.example`); a Sonnet agent wrote the tests, and the coordinator read the whole diff. Evidence:
- `scripts/verify.sh --repeat cue` run by the coordinator in the main checkout: all checks passed, 20 of 20 repeats. 372 tests, 16 of them new (11 in `cues_tests.rs`, 1 in `playback_tests.rs`, 4 in `repl_tests.rs`), plus the 2 config tests from the earlier session.
- Files: one table covers a good file, a missing one, garbage bytes, stereo, 8-bit, float, 8,000 Hz, no samples and data cut short. The 2 s limit is tested at 32,000 vs 32,001 samples (16 kHz) and 88,200 vs 88,201 (44.1 kHz); the volume at 0, 0.01, 1 and 1.01.
- Listening cue: with the wait removed from `listen_once`, both `..._plays_the_cue_out_before_the_recorder_starts` tests fail; restored, they pass. They show the recorder has not started when the cue is asked for and that `[listening]` comes no sooner than the cue's length later.
- Idle cue on a tap: the existing `a_tap_while_speaking_stops_the_voice_keeps_the_answer_and_listens` shows thinking, speaking, listening with no idle between, and the `CueSequence` table shows that order plays no idle cue.
- `git check-ignore data/sounds/idle.wav` matches `/data/` in `.gitignore`.
- The off switch (`JARVIS_CUE_VOLUME=0`) loads nothing, so every cue has length zero and `listen_once` does not sleep. That a zero-length cue adds no wait is by construction; a test cannot observe a sleep that does not happen.

Not ticked, and only the Pi can show them: that a real clip sent to speech-to-text does not contain the listening cue (second half of that criterion), and Darren hearing all six in a conversation. Also untested there: whether the USB speaker clips a cue that follows silence. The audio device is now opened at startup when a cue loaded, instead of at the first answer; a device that fails to open is reported at the first answer, as before.
