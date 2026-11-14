# 05: Note facts by itself while idle

Status: ready-for-agent
Stories: US6
Blocked by: 03
Assignee: unassigned
Owns: src/features/memory/** for the noting pass, src/features/assistant/** for a bounded extraction run, src/interfaces/cli/repl/idle.rs, the storage needed to mark a turn examined, and their tests under tests/unit/

**What to build:** Spec D7. During idle time, beside the existing idle embedding work, Jarvis examines completed turns it has not examined yet and asks Gemma for lasting facts Darren stated about himself. The extraction prompt lives in a text file like the system prompt, sees only Darren's side of the turn, and must return a strict, parseable form (no facts is a valid answer). Output is untrusted: each candidate passes the fact limits of ticket 03, exact duplicates of a listed fact are skipped, and anything unparseable is discarded. Each kept fact is inserted as a noted fact and shown as a "Noted: …" notice (terminal only). User input always wins: the pass is one more branch of the idle `select!`, safe to drop at any `.await`, and a dropped pass leaves the turn unexamined for later. Dropped and failed turns are never examined. The run has its own named timeout and never uses tools. High effort: concurrency and a second kind of model run.

## Acceptance criteria
- [ ] Streamed model tests: a turn stating a fact yields a noted fact and a notice; a turn with none yields nothing; malformed output yields nothing and no error shown to the user.
- [ ] A typed line or a tap during the pass is handled at once, and the turn is examined on a later idle.
- [ ] A turn is examined at most once after a completed pass; dropped, failed and already-examined turns are skipped.
- [ ] The extraction request contains none of Jarvis's own answer text.
- [ ] The Verification list passes for the workspace.
- [ ] The report says what only the Pi proves (what a real Gemma notes from real talk, how long a pass takes, whether it delays the next answer).

## Comments
