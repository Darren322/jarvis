# Visual interaction with Jarvis

Status: active
Resume here: tickets 01 (macroquad spike), 03 (workspace and `orb-protocol`), 04 (recorder audio tap) and 05 (speech playback level) can run in parallel; ticket 02 (VAD spike) needs Darren's voice.

Settled with Darren in a grilling session on 2026-11-07. This file is the source of truth for the epic. Tickets are in `issues/`; their `Status:` lines are the progress record.

## Problem Statement

Jarvis has a voice path (push-to-talk, `/listen`) but the 5" touchscreen on the 8 GB Pi shows nothing of it. Darren cannot see whether Jarvis is listening, thinking or speaking, and he must type `/listen` and press Enter to talk. He wants a screen he can look at and tap.

## User Stories

1. As Darren, I want to see what Jarvis is doing (idle, listening, transcribing, thinking, speaking) as an orb on the Pi's 5" touchscreen, so that I know its state at a glance.
2. As Darren, I want to see the orb react to Jarvis's voice while it speaks, so that it feels like the voice comes from the orb.
3. As Darren, I want to see the orb react to my voice while it listens, so that I know the microphone hears me.
4. As Darren, I want to tap the orb to make Jarvis listen to me, at any time, even while it is speaking or thinking, so that I can interrupt without a keyboard.
5. As Darren, I want Jarvis to send what I said when I stop talking, without pressing anything, so that talking is hands-free once I have tapped.
6. As Darren, I want to tap again while it is listening to send right away, so that I do not wait for the silence to be detected.
7. As Darren, I want to see a clear offline look when Jarvis is not running, so that I can tell a stopped Jarvis from a frozen screen.
8. Later: As Darren, I want to say "Hey Jarvis" to start listening hands-free, so that I do not need to touch the screen.
9. Later: As Darren, I want to run Jarvis without a terminal (headless), so that the orb is the whole device experience.
10. Later: As Darren, I want the orb's colour to show Jarvis's mood (blue calm, gold engaged, red angry), so that its tone is visible.

## Solution

A separate program, `jarvis-orb`, draws an original orb full-screen on the 800x480 touchscreen. Jarvis serves a Unix socket. It sends the assistant state and a loudness level about 30 times a second. The orb sends taps. Jarvis decides what a tap means from its real state, and ends a recording on silence using a speech-detection (VAD) model. Jarvis stays a terminal program for now.

## Implementation Decisions

| # | Decision |
| --- | --- |
| D1 | **Look.** An original abstract orb. It is inspired by a generic plexus network sphere and by Raphael from "That Time I Got Reincarnated as a Slime", and copies neither. It is a sphere of points and links, glyph rings that use invented symbols, a warm core, and light rays. No text on screen. |
| D2 | **Look per state.** Idle: dim blue, slowly turning, breathing. Listening: nodes pulse with the mic level. Transcribing: a glyph ring spins. Thinking: counter-rotating rings and a glowing gold core. Speaking: rays and node motion follow speech loudness. |
| D3 | **Offline look.** A dim grey breathing orb. It ignores taps and reconnects every few seconds. No crash and no popups. |
| D4 | **Program.** A separate Rust program, `jarvis-orb`, using macroquad 0.4.16 pinned exactly (it uses `miniquad =0.4.11`, whose Wayland backend handles `wl_touch`). Set `linux_backend` to Wayland, because miniquad's default is X11Only. Full screen on the 800x480 DSI-2 output with the cursor hidden. labwc 0.20.2 already maps the touchscreen with `mouseEmulation="yes"`, so taps also arrive as clicks. Bevy is the fallback only if the spike (01) fails; it has an open Pi 5 crash issue, #18867. Darren approved macroquad and the workspace on 2026-11-07; pin exactly and check the aarch64 build. |
| D5 | **Repo.** A Cargo workspace. The root `jarvis` crate is unchanged, plus `crates/orb-protocol` (the shared message types, serde only) and `crates/jarvis-orb` (the macroquad window). Jarvis never depends on macroquad. |
| D6 | **Transport.** A Unix socket at a fixed path. The path is a ticket detail (07): under `$XDG_RUNTIME_DIR` (`/run/user/1000`) or the repo `data/`, with the reasoning given. One JSON message per line (serde_json, already a dependency). Jarvis to orb: the state, and a 0 to 1 level about 30 times a second with a source of speech or mic. Orb to Jarvis: `Tap`. Jarvis is the server; the orb reconnects. |
| D7 | **Jarvis decides what a tap means,** from its real current state. The orb only reports taps, because it sees state late and would race. Idle: start listening (the tap path ends on silence). Speaking: stop speaking, then listen. Thinking: drop the in-flight answer (it is never archived, per the AGENTS History rule), then listen. Listening: send now. Transcribing: ignore. |
| D8 | **Taps join stdin** as a second input source in the REPL, so T6's listening path is reused, not copied. A small new module, `src/interfaces/orb/`, serves the socket. It follows the AGENTS "Second interface" rule: it shares only `features/` with the CLI. |
| D9 | **Loudness is computed in Jarvis,** 0 to 1, about 30 Hz, from audio actually being played (speech) and from mic chunks while listening. The orb only animates. |
| D10 | **Ending on silence uses a proper VAD model,** Darren's choice over a loudness threshold. Spike 02 compares Rust options (for example a Silero VAD wrapper and the WebRTC VAD) on Darren's real USB mic, for accuracy and CPU on the 8 GB Pi. The chosen crate needs Darren's approval. VAD is just another `stop` future for `Listener::listen`. The recorder must expose its audio chunks to observers (mic level plus VAD); one change (04) serves both. The typed `/listen` keeps Enter; adding VAD there is optional and later. |
| D11 | **Running Jarvis.** It stays a terminal program for now (`run.sh` started by hand). Taps work while it runs. Headless is US9, later. |
| D12 | **Launch.** A labwc autostart entry starts `jarvis-orb` at desktop login. `~/.config/labwc/autostart` does not exist yet. Changing the Pi needs Darren's OK (push-to-talk D10). |
| D13 | **Wake word (US8)** reuses the VAD and `Listener` path. It is its own later spike: verify openWakeWord's "hey jarvis" model and survey Rust-native detectors. The detector runs on the 8 GB Pi. |
| D14 | **Supersedes the 2D character.** The earlier "Jarvis animation (2D character, web page in Chromium)" idea in the roadmap and in push-to-talk "Out of scope" is replaced by this orb. No web server crate is needed. |
| D15 | **Approvals (2026-11-07).** Darren approved `thiserror` (`=2.0.21`, as in the root crate) for `orb-protocol`, and member-crate tests in `crates/<name>/tests/` (one line added to AGENTS.md Testing). |

## Code facts these tickets build on

- `src/features/state.rs`: `AssistantState { Idle, Listening, Transcribing, Thinking, Speaking }`.
- `src/features/listening/listener.rs`: `Listener::listen(stop: impl Future, transcribing: oneshot::Sender<()>) -> Result<Heard, ListenError>`, where `Heard` is `Text` or `DidntCatch`. The stop signal is a parameter, so any trigger can supply one.
- `src/features/listening/mod.rs`: `Recorder` (an `arecord` child process, read in chunks).
- `src/interfaces/cli/repl/{mod,listen}.rs`: the REPL loop and `/listen`.
- `src/features/speech/`: synthesis and playback (rodio).
- `Cargo.toml`: a single crate today, no workspace.

## Testing Decisions

- Follow the AGENTS Testing section. Unit tests of the root crate live in `tests/unit/` mirroring `src/`, registered with the `#[path]` mod. Workspace member crates under `crates/` keep their tests in `crates/<name>/tests/`; `tests/unit/` mirrors the root crate's `src/` only.
- Test each limit at its boundary: last accepted value and first rejected value (for example a level of 0.0 and 1.0 accepted, and out-of-range values clamped or rejected as the ticket decides; the longest accepted message line and one byte more). Use tables.
- No real microphone, speaker, screen or network. Recorder tests use a fake script program, as the existing recorder tests do. The socket server is tested over a temporary Unix socket path cleaned up by a `Drop` fixture. Any HTTP stays on Wiremock.
- Use `#[tokio::test(start_paused = true)]` for timing: the 30 Hz level cadence, reconnect delay, and silence timeouts. No wall-clock sleeps.
- Test tap mapping once, as a pure function of the real state, in the layer that owns it. The server tests cover only the wire format and connection handling.
- The rendering in `jarvis-orb` is checked on the Pi by eye (ticket 10). Its state-to-look mapping and the reconnect logic are plain functions and are unit-tested.
- Local tests do not prove Pi behaviour. Say so in each report.

## Out of Scope

- A phone or remote web UI.
- Any cloud service or cloud request.
- A 2D character face (superseded by the orb, D14).
- Headless Jarvis, wake word and emotion colour (US8 to US10): kept as `needs-triage` ticket 11 until they are designed.
- VAD on the typed `/listen` (optional, later).

## Further Notes

- The emotion source for US10 is not designed. A probable route is Gemma tagging its own replies, kept in a separate field and not mixed into the state enum.
- Phoneme or viseme lip-sync is not needed for an orb.
- Dependencies in this epic need Darren's approval before use: macroquad (approved 2026-11-07), the VAD crate (ticket 02 recommends one).
