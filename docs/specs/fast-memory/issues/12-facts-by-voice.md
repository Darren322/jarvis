# 12: Listing and forgetting facts by voice

Status: done
Stories: US7
Blocked by: 06
Assignee: Sonnet agent (2026-11-21)
Owns: `src/features/control/**`, `src/interfaces/cli/repl/mod.rs` (only where memory-command notices are shown), `src/interfaces/cli/repl/speech.rs` (only if `speak_lines` needs a change), `src/features/assistant/note_facts.txt`, and their tests under `tests/unit/`

**Why:** Found on the Pi on 2026-11-21, where Jarvis runs as a service with no terminal, so only voice works. D9 says facts can be listed and removed "typed or spoken"; 06 delivered only the typed half.
- Darren said "Forget that my sister name is Mia." (transcribed exactly). The fact was "The user has a sister named Mia." Jarvis answered "I could not find one memory or fact matching that detail; nothing was forgotten.", printed only, so he heard nothing. Cause: "forget <detail>" matches facts by whole-phrase substring.
- `/memory facts` has no spoken form, and no memory-command reply is spoken.
- Jarvis noted "The user has an interview on next Monday.", which is not a lasting fact.

**What to build (coordinator's proposal, Darren asked for it to work by voice):**
1. **Word matching for facts in "forget <detail>".** Drop a leading "that" and filler words (a short named list: that, my, me, the, a, an, is, are, was, of, about, please, called, named, name...; choose and test it). Each remaining word, case-insensitive with punctuation ignored, must match the start of a word in the fact ("name" matches "named", "sis" matches "sister"); at least one word must remain. Memories keep their existing matching. The rule "exactly one match across memories and facts, else nothing" is unchanged.
2. **Spoken replies.** When speech is on, the replies of fact commands ("forget that", "forget <detail>", listing, `/memory forget`) are spoken the way "Got it." is (`speak_lines`), as well as printed.
3. **A spoken list.** Phrases such as "what facts do you have", "list my facts", "list facts", "what have you noted" (with the usual "hey Jarvis", "please" and punctuation handling) list the facts like `/memory facts`. Each line reads well spoken and printed, e.g. "Fact 3, noted: The user has a sister named Mia." An empty list says so.
4. **Prompt:** `note_facts.txt` also excludes plans, appointments and anything tied to a date ("an interview next Monday").

## Acceptance criteria
- [x] "Forget that my sister name is Mia." removes "The user has a sister named Mia."; "Forget that my sister name is Maya please." removes nothing and says so; a detail matching two facts removes nothing. Table test of the word rule, including "name"/"named" and a filler-only detail.
- [x] With speech on, a fact command's reply is spoken (fake `SpeechControl`), and with speech off it is only printed.
- [x] The spoken list phrases parse to the facts listing (parse table with near-misses that stay prompts, e.g. "what facts do you know about the moon"), and the listing reads "Fact <n>, <stated|noted>: <text>".
- [x] The prompt file names dated plans as not to note. `scripts/verify.sh` passes.

## Comments

2026-11-21 (coordinator): built by a Sonnet agent in the main checkout; the coordinator read the diff and ran `scripts/verify.sh`: all checks passed. Committed at Darren's standing OK.
- **Word rule** (`control/fact_match.rs`, `detail_matches_fact`): lowercase, split on anything but letters, digits and apostrophes, drop a possessive "'s", drop the filler words that, my, me, i, the, a, an, is, are, was, of, about, please, called, named; at least one word must remain and each must begin a word of the fact. Memories keep whole-phrase matching; "exactly one match across both" is unchanged. Spot-check by the agent: a substring test in place of the prefix test failed the table ("ister"); restored.
- **Spoken replies:** `MemoryCommand::is_spoken()` (exhaustive) is true for the facts listing, `/memory forget <n>`, "forget that" and "forget <detail>"; the REPL prints them and, with speech on, says them through `speak_lines`. `/memories`, `/forget <id>`, import, status and rebuild stay print-only.
- **Spoken list:** "what facts do you have", "what facts have you saved", "what facts have you noted", "what have you noted", "list my facts", "list the facts", "list facts", "list all my facts", "show my facts", "show me my facts" (after "hey Jarvis", "please" and the like). Longer lines ("what facts do you know about the moon") stay prompts. Listing lines now read "Fact <n>, <stated|noted>: <text>".
- **Prompt:** one line added to "Do not note": "plans, appointments or anything tied to a date (an interview next Monday),".
- **Changed expectations:** three listing assertions in `memory_tests.rs` for the new line format.
- **Only the Pi proves:** how the spoken replies and list sound, real transcriptions against the phrase list, and what Gemma notes with the new prompt line.
- 2026-11-21 (coordinator): committed as `390e71d`, pushed and deployed to the 8 GB Pi: Jarvis idle, `git pull`, `cargo build --release --locked` (46.7 s), `systemctl --user restart jarvis` at 17:14; active, schema still v6, the three facts unchanged. Waiting for Darren's voice check.
- 2026-11-21 (coordinator), after Darren's voice check ("'List my facts' didnt work btw, but 'what facts do you do you have' worked. Forgetting seems to work ? but i had to repeat twice."). From the Pi log: "list my facts" was transcribed "This my fix" and "I said lease my fax", so it never reached the parser; "What facts do you have?" was transcribed cleanly three times and listed the facts. The first forget was transcribed "Can you forget that my sister's name is Mia? you": not a command, so it went to Gemma, which answered "I can certainly disregard that detail", a false claim. The second ("Forget that my sister name is Mia.") removed the fact. Fixed by the coordinator: forget phrases now skip "can you", "could you", "would you", "will you" (with the spoken prefixes, in any order; `strip_request_prefixes`), "you" joins the filler words (Whisper's stray trailing "you"), and the system prompt says Jarvis cannot save or forget anything itself and names the phrases that do. Parse rows for the transcribed line and near-misses; spot-checked both changes (each broken, its test failed, restored); `scripts/verify.sh`: all checks passed. "List my facts" is left as it is: the misheard forms are not worth matching, and "what facts do you have" is the phrase to use.
- 2026-11-21 (coordinator): closed by Darren after his voice check of `5b16e6a` ("voice check of 5b16e6a is good").
