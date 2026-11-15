# 06: Retire jarvis-orb

Status: ready-for-human
Stories: US1
Blocked by: 05
Assignee: unassigned
Owns: `crates/jarvis-orb/` (delete the directory); the `jarvis-orb` line in the workspace `members` list of `Cargo.toml`; `Cargo.lock`; the Unix socket server code, its config and its tests in `src/interfaces/orb/`, `src/config/orb.rs` and `tests/unit/`
Traces to: D13

**Waiting on:** Darren has to see the React Orb on the Pi and say so in Comments. Until then, nothing is deleted.

**What to build:** Delete `crates/jarvis-orb` and remove it from the workspace members. `crates/orb-protocol` stays. The macroquad and miniquad packages leave `Cargo.lock`.

## Acceptance criteria
- [ ] Darren's sign-off on the Pi Orb is recorded in Comments, with the date.
- [ ] `crates/jarvis-orb/` does not exist.
- [ ] The Unix socket server is removed (kept until now by ticket 03). `grep -r UnixListener src/` finds nothing.
- [ ] `grep -c macroquad Cargo.lock` prints 0, and the same for `miniquad`.
- [ ] `cargo check --locked --offline --all-targets` passes after the lockfile update, and `cargo check --locked --offline --workspace` passes.
- [ ] Mentions of `jarvis-orb` in other docs are listed in Comments for the coordinator. This ticket does not edit files outside its Owns.
- [ ] `scripts/verify.sh` passes.

## Comments
