# Local Supertonic speech output

Jarvis uses one local Python worker for Supertonic 3 INT8 with the official M5
voice, and a retained Rust audio output for playback. Python performs inference
only. Assistant, Rig, conversation history, and the archive remain text based.

Speech is optional. Jarvis prints a successful final answer before speaking it.
Speech failures leave text chat usable and never retry the model or archive save.
There is no runtime download, cloud fallback, microphone, or speech queue.

## Raspberry Pi setup

The deployment target is Raspberry Pi 5 8 GB, aarch64 Debian 13, Python 3.13.
Existing user-reported worker and playback smoke tests are recorded in the
[Phase 15B handoff](../../JARVIS_PHASE_15B_IMPLEMENTATION_HANDOFF.md). The integrated
App lifecycle and current changes still require validation on that Pi.

Check the existing interpreter/environment before installing anything. Rust
playback compilation on Debian needs `libasound2-dev` and `pkg-config`; a new
Python environment needs `python3-venv`. Install missing prerequisites during
deployment, never during chat startup.

Keep runtime assets outside the repository. The existing setup script requires
the researched INT8 source bundle and official `M5.json` as inputs:

```sh
export JARVIS_TTS_SOURCE_MODEL_DIR=/absolute/path/to/original-int8-bundle
export JARVIS_TTS_SOURCE_M5_JSON=/absolute/path/to/original/M5.json
export JARVIS_TTS_RUNTIME_ROOT="$HOME/.local/share/jarvis/tts"
export JARVIS_TTS_VENV_DIR="$JARVIS_TTS_RUNTIME_ROOT/venv"
bash scripts/tts/setup_pi.sh
```

This is a provisioning command: pip and the pinned official voice converter
need network access. The script installs `requirements.lock` with required
hashes, verifies the source assets against `assets.lock.json`, converts M5 alone
to `voice.bin` with speaker ID 0, verifies the prepared files, and writes a local
`manifest.json`. Do not set the prepared output directory to the original
research directory: preparation replaces an existing output bundle. A fresh
isolated setup and the Python 3.13/aarch64 wheel hashes require Pi verification.

For a staged offline installation into an existing isolated environment:

```sh
"$JARVIS_TTS_PYTHON" -m pip install --no-index --find-links /absolute/path/to/wheelhouse \
  --require-hashes -r scripts/tts/requirements.lock
```

The wheelhouse must contain every locked compatible wheel. Asset preparation
currently fetches the pinned converter; the setup script does not offer a fully
offline provisioning mode. Normal Jarvis inference loads only local files.

## Enable speech
Keep the existing required `LOCAL_LLM_BASE_URL`, `LOCAL_LLM_HEALTH_URL`, and
`LOCAL_LLM_MODEL` settings in the checkout's local `.env` file. Add these speech
settings there once:

```dotenv
JARVIS_TTS_PYTHON="$HOME/.local/share/jarvis/tts/venv/bin/python"
JARVIS_TTS_MODEL_DIR="$HOME/.local/share/jarvis/tts/models/supertonic-3-int8-m5"
JARVIS_TTS_WORKER=scripts/tts/supertonic_worker.py
JARVIS_TTS_THREADS=2
```

Start Jarvis from the checkout with `./run.sh`. It runs `cargo check --locked`
first, then starts `cargo run --locked` if the check succeeds. You can also call
the script by its full path from another directory.

Omitting `JARVIS_TTS_MODEL_DIR` disables speech. The interpreter must be an
absolute path. The default worker path is `scripts/tts/supertonic_worker.py`
resolved at startup; deploy the adjacent `assets.lock.json` with that script.
Threads default to 2 and must be 1–4. `JARVIS_AUDIO_DEVICE` optionally chooses an
exact CPAL device identifier or device description; omit it to use the default
output. On the locked CPAL ALSA backend, identifiers have the form `alsa:<PCM>`.
For the USB route reported on Darren's Pi, select it explicitly in `.env`:

```dotenv
JARVIS_AUDIO_DEVICE="alsa:plughw:CARD=Device,DEV=0"
```

This selects the enumerated USB PCM route; a plain ALSA route without the `alsa:`
prefix is not a CPAL identifier. Device descriptions can be shared by multiple
PCM routes, so prefer an identifier when choosing a particular route. Invalid
optional settings warn once and preserve text chat. Resource initialization
starts only for an eligible answer.

Audio opening tries the selected device's initial stream configuration, then
Rodio's supported configurations on that same device if needed. Opening failures
include the underlying CPAL error. After a speech failure, exit and restart
Jarvis before retrying; speech stays disabled for the failed session.

Each spoken answer begins with a quiet one-second 440 Hz cue, immediately
followed by the original speech. This adds one second before the words. The
cue was selected after the same M5 WAV lost its opening words with `aplay` and
with a silent lead-in, but played fully with this tone before it. Integrated
Pi playback still needs confirmation after deploying the patch. `/stop` and
new-prompt interruption stop the cue and speech together.

The existing generation settings are English, speaker ID 0, speed 1.0, and eight
steps. These preserve the current worker rather than the earlier speed 1.05
proposal. Their latency and voice quality need an integrated Pi measurement.

## Controls and limits

- `/stop` stops current speech. During startup/synthesis it kills and reaps the
  worker; during playback it retains the healthy worker and output device.
- A new prompt stops and cleans up speech before one new model request.
- `/reset` stops speech before clearing active conversation context.
- `/exit`, EOF, and terminal errors run explicit speech shutdown.
- `/stop` while idle never calls the model. Model requests retain their existing
  serial behavior; this command does not cancel a model request or tool action.

Speech input is at most 4,096 UTF-8 bytes. Larger answers remain visible and skip
speech. IPC requests are bounded to 32 KiB and responses to 4 KiB. Startup has a
60 second deadline and the entire synthesis write/result operation 30 seconds.
Blocking audio initialization is bounded to 60 seconds and WAV decoding to 30
seconds; their handles remain owned if a deadline expires.
Audio must be nonempty 44,100 Hz mono 16-bit PCM WAV, at most 16 MiB and 120
seconds. Playback polls every 20 ms with a deadline of validated speech duration
plus the one-second cue and two seconds of grace, at most 123 seconds. Temporary
audio is private and ephemeral.

Rodio source completion does not prove the hardware has drained; see the
[Rodio 0.22.2 output API](https://docs.rs/rodio/0.22.2/rodio/stream/struct.DeviceSinkBuilder.html).
Typed exits
have cleanup paths; abrupt signals and crashes are not verified graceful exits.
Started [Tokio 1.53.1 blocking tasks](https://docs.rs/tokio/1.53.1/tokio/task/fn.spawn_blocking.html)
cannot be forcibly aborted. Unconfirmed resource
settlement is reported and disables further speech rather than claiming cleanup.

## Pi acceptance still required

Use public text only. Record the interpreter/package versions, locked asset
hashes, chosen audio device and buffer settings, and cold/warm timing and RSS.
Confirm two ordinary answers reuse the worker PID, `/stop` during startup,
synthesis and playback, a new prompt interrupt, `/reset`, `/exit`, EOF, optional
speech disabled, and speaker unplug/error behavior. Confirm no live or zombie
worker remains after controlled shutdown and temporary WAVs are removed.

Replay the same generated M5 WAV through retained output on cold use, repeatedly,
and after idle. Listen for the complete first word and measure audible stop
latency. The historical standalone playback successes do not establish that
intermittent USB/ALSA first-word clipping is resolved.
