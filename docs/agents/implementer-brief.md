# Rules for an implementing agent

The coordinator points every implementing agent here instead of repeating these in each brief. The brief itself adds the ticket, the files you own and the decisions already made.

1. **Read first:** `AGENTS.md` in full, your ticket, the spec decisions it cites, then the code and tests you will change.
2. **Stay inside your files.** Edit only the paths the brief lists. Ticket files, specs, `docs/`, `notes/`, `Cargo.toml` and `Cargo.lock` belong to the coordinator unless the brief says otherwise.
3. **Leave git alone:** work in the checkout you were given and hand back uncommitted changes. The coordinator commits.
4. **Stop and report** when a command fails, an approval is missing, or the brief contradicts the code. A workaround hides the problem from the person who can fix it.
5. **Library facts come from the locked source** in `~/.cargo/registry/src/`, with the version in `Cargo.lock`.
6. **Verify with one command:** `scripts/verify.sh`. For any new test of a `select!` race, cancellation or timing, also run `scripts/verify.sh --repeat <test name pattern>` (20 runs).
7. **Spot-check each new boundary test:** break the limit by one, see the test fail, restore it.
8. **Report facts.** Start with one status line: DONE, DONE WITH GAPS or BLOCKED. Then the files changed, the public items and constants added with their values, the tests added, the tail of `scripts/verify.sh`, any existing test expectation you changed and why, and what you did not verify. What only the Pi can prove goes in its own list.
