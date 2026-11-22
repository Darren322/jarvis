# 08: When a timer ends: the sound and Jarvis's own model run

Status: in-progress
Stories: US3, US7
Blocked by: 07
Assignee: Claude
Owns: `src/features/speech/cues.rs`, `src/config/cues.rs`, `src/features/assistant/**` (the run's no-tool setting, `preamble.txt` only if Darren agrees), `src/interfaces/cli/repl/**`, `src/interfaces/cli/input.rs`, `src/features/conversation/**` (only the 30-minute rule's hook, if needed), `src/app.rs` (only sound loading and the event wiring), `tests/unit/features/speech/**`, `tests/unit/config/cues_tests.rs`, `tests/unit/features/assistant/**`, `tests/unit/interfaces/cli/**`. The WAV file itself (`data/sounds/` on the Pi, not in git) is Darren's to pick (D21); until it is there the cue is a warning and silence.
Traces to: D6, D7, D8, D9, D12

**Coordinator implements (concurrency, cancellation, cross-module wiring).** Do not give this to an agent.

**What to build:**
- **Sound:** a timer-ended cue, a WAV in `data/sounds/` loaded like the state sounds (`CueSounds`, `JARVIS_CUE_VOLUME`; a missing file is a warning and silence, as for the others). It is played when the service's ended event arrives, at once, whatever Jarvis is doing, and it is not a state cue (it does not follow `AssistantState`). Check `Cue` and `CUE_COUNT` before choosing between a new variant and a separate path.
- **The run (D6, D7):** once Jarvis is idle, the REPL starts a model run on its own, read only in the REPL's idle `select!` beside announcements. The prompt is a fixed line from Jarvis ("The timer 'Time to sleep', set at 10:30 pm, has ended.") plus the normal history and facts list. Several ended timers run one after another. A late timer from D9 gets "ended 40 minutes ago while I was off". The answer is spoken, archived and added to the history like any other turn.
- **No tools (D6, D20):** the run may call no tools. Keep the tool list in the request and set tool choice to `none`, exactly as the policy does on the second turn, because changing the list empties the prompt cache (fast-memory 10; AGENTS.md "Policy"). The policy's rules are not loosened for ordinary turns.
- **The 30-minute rule (D8):** a timer run counts as a prompt for fast-memory 09: after 30 minutes of quiet it starts a fresh conversation first, and its answer counts as the last answer.
- **Busy (D7):** while recording, thinking or speaking, the run waits for idle; the row already shows as ended.

## Acceptance criteria
- [ ] The run is scripted with `MockCompletionModel::from_stream_turns`; a test checks that the request lists all 8 tools with tool choice `none`, and that a scripted tool call in the answer is rejected (no tool runs, a label like "turn on the oven" changes nothing).
- [ ] While Jarvis is recording, thinking or speaking, no run starts; the cue still plays at once; the run starts when idle returns. Tested for each busy state.
- [ ] The prompt carries the label and the set time; a late timer carries how late it was; an unlabelled timer reads naturally. Table-tested.
- [ ] After 30 minutes of quiet (29 minutes 59 seconds vs 30 minutes, paused clock) a timer run starts a fresh conversation; before that it joins the current one. The answer is archived and counts as the last answer.
- [ ] Two timers ending together give two runs in order, never overlapping, and a tap or spoken sentence from Darren while one is waiting wins (`biased;`).
- [ ] A missing cue file warns and stays silent; `JARVIS_CUE_VOLUME=0` loads and plays nothing. Config and cue tests updated, with the file name listed in `cues.rs`.
- [ ] All race and timing tests pass `scripts/verify.sh --repeat <pattern>` (20 runs).
- [ ] No `unwrap()`; `scripts/verify.sh` passes.
- [ ] Pi check (Darren's ears): a 1-minute version of T1 or T2 ends with the sound and a spoken line that fits the label ("Time to sleep" sounds like a reminder, not a beep); one that ends while he is talking waits until Jarvis is idle; a restart with a running timer still ends it on time; a timer that ended while Jarvis was off (stop the service, wait, start) runs late and says so. Gemma's wording can only be judged on the Pi.

## Comments
