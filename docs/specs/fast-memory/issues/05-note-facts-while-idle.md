# 05: Note facts by itself while idle

Status: done
Stories: US6
Blocked by: 03
Assignee: Sonnet agent, high effort (2026-11-21)
Owns: src/features/memory/** for the noting pass, src/features/assistant/** for a bounded extraction run, src/interfaces/cli/repl/idle.rs, the storage needed to mark a turn examined, and their tests under tests/unit/

**What to build:** Spec D7. During idle time, beside the existing idle embedding work, Jarvis examines completed turns it has not examined yet and asks Gemma for lasting facts Darren stated about himself. The extraction prompt lives in a text file like the system prompt, sees only Darren's side of the turn, and must return a strict, parseable form (no facts is a valid answer). Output is untrusted: each candidate passes the fact limits of ticket 03, exact duplicates of a listed fact are skipped, and anything unparseable is discarded. Each kept fact is inserted as a noted fact and shown as a "Noted: …" notice (terminal only). User input always wins: the pass is one more branch of the idle `select!`, safe to drop at any `.await`, and a dropped pass leaves the turn unexamined for later. Dropped and failed turns are never examined. The run has its own named timeout and never uses tools. High effort: concurrency and a second kind of model run.

## Acceptance criteria
- [x] Streamed model tests: a turn stating a fact yields a noted fact and a notice; a turn with none yields nothing; malformed output yields nothing and no error shown to the user.
- [x] A typed line or a tap during the pass is handled at once, and the turn is examined on a later idle.
- [x] A turn is examined at most once after a completed pass; dropped, failed and already-examined turns are skipped.
- [x] The extraction request contains none of Jarvis's own answer text.
- [x] The Verification list passes for the workspace.
- [x] The report says what only the Pi proves (what a real Gemma notes from real talk, how long a pass takes, whether it delays the next answer).

## Comments

2026-11-21 (coordinator): built by a Sonnet agent at high effort in a worktree, in parallel with 04; the coordinator read the source (idle loop, storage commit, wiring), merged it onto 04 (one doc-comment conflict in `session.rs`, resolved by hand) and ran `scripts/verify.sh` in the main checkout: all checks passed; `--repeat repl::idle`, `--repeat noting` and `--repeat note_facts`: 20 of 20 each. Committed at Darren's standing OK; not deployed. Darren asked to read the extraction prompt (`src/features/assistant/note_facts.txt`) before it ships.
- **Schema v6:** `ALTER TABLE turns ADD COLUMN facts_examined INTEGER NOT NULL DEFAULT 0 CHECK (facts_examined IN (0, 1)); UPDATE turns SET facts_examined = 1; CREATE INDEX turns_awaiting_facts ON turns(id) WHERE facts_examined = 0 AND outcome = 'completed';`. Every turn archived before v6 counts as examined, so the first idle does not run Gemma over the whole history; `UPDATE turns SET facts_examined = 0` would examine them after all (Darren to decide).
- **Extraction run** (`assistant/note_facts.rs`, `FactExtractor`): a separate Rig agent on the same model with no tools, no preamble and no policy hook; temperature 0, `max_turns(1)`, `NOTE_FACTS_MAX_TOKENS` 160, `NOTE_FACTS_DEADLINE` 30 s. The whole request is one user message: the prompt file with the turn's user text at the end, so its start is the same every run. Output cut short (any finish reason but stop) or over 2,048 bytes is discarded.
- **Parsing and storing** (`memory/noting.rs`, `storage/noting.rs`): lines starting `FACT:` are candidates, at most 5 a turn; anything else, `NONE` included, is ignored. One immediate transaction re-checks that the turn is still open (completed, not examined, its source still active), inserts the candidates that pass the 03 limits and are not a duplicate (trimmed, case-insensitive), and marks the turn examined. A model failure or timeout leaves the turn unexamined; after `MAX_ATTEMPTS_PER_TURN` 3 failures it is skipped until restart (counted in memory, never marked examined).
- **Idle loop** (`repl/idle.rs`): embedding first, then noting one turn at a time, oldest first. Only the model run is raced with input (`biased;`); the store is awaited after it. A failure gives one fixed warning and ends noting for that idle. "Noted: <fact>" and "Dropped to make room for them: …" are terminal notices, not spoken. Known residual: a tap or line landing during the few-millisecond store keeps the facts but loses their "Noted:" lines.
- **Changed expectation:** `archive_tests::rejects_newer_schema...` now uses version 7.
- **Follow-up, not in this ticket:** a newly noted fact changes the facts message, so the next answer rereads the prompt from there (spec D5 accepts this cost; it is more frequent with noted facts). A warm-up after a pass that noted something would hide it.
- **Only the Pi proves:** what a real Gemma notes from real talk and whether it keeps the `FACT:` format; pass duration; which server slot the extraction uses and whether it evicts the conversation's cache; whether dropping the stream stops generation; the reread cost after a new fact; the v6 migration on the real database (back up first).
- 2026-11-21 (coordinator): Darren read the extraction prompt and kept "the user" wording: "I will add user auth via voice and face. So shal lsee". Facts are global today (no owner column); per-person spaces (roadmap 12D) will need one.
- 2026-11-21 (coordinator): deployed to the 8 GB Pi with 03 to 06 at `e2bc1a8` (Darren: "yes build restart etc"). Built while Jarvis ran (45.9 s); stopped it at 16:28:35, backed up the database with `sqlite3 .backup` to `data/jarvis.sqlite3.bak-2026-11-21` (integrity check ok; version 4, 366 turns, 344 memories), started it. After start, read-only: `user_version` 6, 366 turns and 344 memories, 0 facts, all 366 turns `facts_examined = 1`; the screen reconnected. Waiting for Darren's voice checks.
- 2026-11-21 (coordinator): closed by Darren after his voice check of `5b16e6a` ("voice check of 5b16e6a is good").
