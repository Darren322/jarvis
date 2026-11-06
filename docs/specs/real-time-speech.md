# Spec: real-time speech

Settled with Darren on 2026-11-06, after the T1 acceptance check of [push-to-talk](push-to-talk.md) found that speech starts only after the whole answer and that one failure disables speech until restart. This spec comes **before** push-to-talk T2–T6. **Update the Status section at the end of every session.**

## Status

| Ticket | State | Notes |
| --- | --- | --- |
| R0 Measure latency | in progress | Agent adding time to first text and measuring Gemma TTFT and Supertonic RTF on the Pis |
| R1 Speech recovery | in progress | Agent replacing the `disabled` bool with a state enum; thiserror sources; rodio print |
| R2 Chunker | in progress | Agent building the incremental chunker test-first |
| R3 Streaming pipeline | not started | Blocked on R1, R2 |
| R4 Voice decision | not started | Blocked on R0 |

**Resume here:** R0, R1 and R2 can run in parallel; R3 follows R1 and R2.

## Goal

Jarvis starts speaking while Gemma is still generating. Synthesis and playback overlap token generation, which runs on the other Pi. Success is measured as **time from Enter to the first audible word**, aiming for at most 3 s on a short question, with the target fixed after R0's baseline. A failure in one answer never silences later answers.

## Problem (observed 2026-11-06 on the Pi)

- Today the stages run one after another: the full text streams (7–16 s), then the whole answer is synthesized, then it plays.
- An 830-byte answer failed with `speech synthesis failed`, most likely because of the 30 s `SYNTHESIS_TIMEOUT`. The cause is hidden: the `Display` on `SpeechError` is hand-written and drops the source.
- `SpeechOutput::fail_operation` sets `self.disabled = true` permanently (`src/features/speech/output.rs`), so Jarvis must be restarted to get speech back.
- rodio prints `speech output configuration: DeviceSinkConfig {...}` into the prompt.

## Decisions

| # | Decision |
| --- | --- |
| S1 | This spec comes before push-to-talk T2–T6. T0, T1 and T5 stay done. |
| S2 | Measure first: R0 adds time to first text, time to first audio and the synthesis real-time factor; the latency target is fixed from that baseline. |
| S3 | Keep Supertonic unless R0 shows it cannot synthesize faster than real time on the Pi. Only then compare Piper (R4). |
| S4 | Recovery: show the real cause; restart the worker on the next answer; disable after 3 consecutive failures with a clear notice. State is an enum (`Ready`, `Recovering`, `Disabled`), not a bool. |
| S5 | Chunking: the first chunk is cut at the first sentence end or clause break (`,` `;` `:`) once at least about 40 characters have arrived; later chunks are whole sentences; anything longer than about 300 characters is force-split at a word boundary. |
| S6 | Pipeline: text stream, then chunker, then a **bounded** queue, then one synthesis at a time on the single worker while the previous chunk plays, with no gaps. `/stop` and a new prompt clear the queue and the current playback. The 700 ms lead-in noise plays once per answer, not per chunk. |
| S7 | Engineering practice per AGENTS.md: reuse rodio's queueing (`Sink`/`Player` append) rather than a hand-written mixer; no new crates without approval. |

## Tickets

Each ticket lists the files it owns. Code tickets finish with the full verification list in AGENTS.md, including the Python worker test if speech changed.

### R0: measure latency
- Add to the run diagnostics: time to first streamed text, and time to first audio (when playback of an answer starts). Mark them `Unavailable` when not observed, never 0.
- On the Pi: measure Supertonic's real-time factor (synthesis seconds per audio second) for 3 sentence lengths. Measure Gemma's time to first token for a short and a long prompt.
- Output: the numbers in this spec's Status notes and `notes/SPEECH_LATENCY.md`.
- Owns: `src/features/assistant/run_observer.rs`, `run_report.rs`, `src/interfaces/cli/**`, and their tests. If time to first audio needs a signal from `features/speech/`, report the needed API instead of editing it (R1 owns that module).

### R1: speech recovery
- Replace the `disabled` bool with a state enum (S4). Derive errors with `thiserror` and `#[source]`, so the CLI warning shows the cause chain. On a failure, terminate the worker and start a fresh one on the next answer. After 3 consecutive failures, move to `Disabled` with a notice. A success resets the count.
- Silence or redirect rodio's device-configuration print.
- Tests: recovery after one failure, disabled after the third consecutive failure, and a success between failures resets the count (a boundary table for 2/3/4).
- Owns: `src/features/speech/**` (except a new `chunker.rs`) and `tests/unit/features/speech/**`.

### R2: chunker
- A pure, incremental chunker: push streamed text deltas in, get speakable chunks out, and flush the rest at the end. Rules are S5. It handles abbreviations and decimals ("Dr.", "3.5") without splitting, and Markdown that the speech cleanup already strips.
- Tests: a table covering 39/40/41 characters for the first chunk, 300/301 for the force split, sentence and clause boundaries, and deltas split mid-word.
- Owns: `src/features/speech/chunker.rs` and `tests/unit/features/speech/chunker_tests.rs`, plus its `mod` line in `src/features/speech/mod.rs`.

### R3: streaming pipeline
- Wire the turn stream into the chunker, a bounded queue, the worker and gapless playback (S6). `/stop` and a new prompt clear everything. Time to first audio comes from R0's diagnostics.
- Pi acceptance: Darren hears speech start while the text is still streaming; a long story plays fully; `/stop` cuts it off.
- Owns: decided when R1 and R2 land.

### R4: voice decision
- If R0's real-time factor is at most about 0.5, keep Supertonic and close this ticket. Otherwise, compare Piper on the Pi for speed and quality, judged by Darren.
