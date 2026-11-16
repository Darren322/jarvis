# 10: A tool run should not empty the prompt cache

Status: in-review
Stories: US1
Blocked by: None
Assignee: coordinator (2026-11-16)
Owns: `src/features/assistant/policy_hook.rs` and the tests that assert on the second request

**Why:** Seen in Darren's session on the 8 GB Pi on 2026-11-16 (Jarvis's own `Model stage` lines). After any run that calls a tool, the next requests reuse only 476 cached tokens, which is the system prompt, and Gemma reads the whole conversation again at about 58 tokens a second:

| What Darren said | Turn | `cached_input` | Wait |
| --- | --- | --- | --- |
| "Bring up the stats again please" (after a run with no tool) | 1 | 793 of 809 | 1.1 s |
| the same run, after the tool call | 2 | 476 of 699 | 5.1 s |
| "You may close the panel" (the next run) | 1 | 476 of 871 | 8.1 s |
| "Bring up the stats." (after another tool run) | 1 | 476 | 9.6 s, then the Panel appears |
| "Bring up the stats." (again) | 1 | 476 | 10.7 s |

So the Panel took about 10 s to appear, and the wait grows with the conversation. Darren: "it seem abit slow to display the panel".

**Likely cause (read in the code, not yet measured):** `JarvisPolicyHook` sends the second turn of a tool run with no tools (`active_tools` empty, `ToolChoice::None`, `policy_hook.rs`). Gemma's chat template writes the tool list near the start of the prompt, so the second turn's prompt differs from the first right after the system prompt. The slot then holds the no-tools prompt, and the next run, which offers tools again, matches it only up to the same point.

**To find out first (read-only against the Gemma server; `docs/agents/pi.md`):**
- Confirm the cause: send the same conversation with and without the tool list and compare `cached_input`.
- Does a request with the tool list kept and `tool_choice: "none"` render the same prompt start in llama.cpp? If so, the second turn can keep the tools in the prompt while the policy still refuses a second call.
- If not: is the second turn landing in the same slot as the first, and would pinning the two kinds of request to the two slots keep both prompts cached?

**Constraints:** the policy rules stay (two model turns at most, one tool call at most, tools only on the first turn); only how the second request is worded may change. The warm-up of ticket 08 must still match the next request.

## Acceptance criteria
- [x] The cause is confirmed or replaced by a measurement, written in Comments.
- [ ] After a tool run, the next run's first turn reuses the conversation from the cache (`cached_input` close to `input`), measured on the Pi with `scripts/pi/bench_first_text.py` on a script that includes tool calls.
- [ ] The second turn of a tool run reuses the first turn's prompt from the cache.
- [x] A second tool call is still refused, and tested.
- [x] `scripts/verify.sh` passes.

## Comments

- 2026-11-16 (coordinator): cause confirmed and fixed in the main checkout.
  - **Measured (read-only, on the 16 GB Pi against Gemma, throwaway conversations of about 1,200 tokens with the three tool definitions, `max_tokens` 1; the script was removed afterwards).**

    | Second turn sent as | Second turn: cached, wait | Next run's first turn: cached, wait |
    | --- | --- | --- |
    | no tools, choice `none` (as before) | 602 of 1,116, 9.6 s | 602 of 1,273, 12.6 s |
    | tools kept, choice `none` | 1,217 of 1,254, 0.8 s | 1,254 of 1,273, 0.5 s |
    | tools kept, choice `auto` | 1,226 of 1,263, 0.8 s | 1,263 of 1,282, 0.5 s |

    So the missing tool list was the cause, and llama.cpp writes the tool list into the prompt whatever the tool choice is.
  - **Fix.** `JarvisPolicyHook` now sends the second request with the same `active_tools` as the first and `ToolChoice::None` (one line plus a comment). The policy is unchanged: `validate_turn` still refuses a tool call on the second turn, and `a_second_tool_call_is_still_rejected` still passes. `AGENTS.md`'s Policy rule now says "tool calls only on the first turn" and why the list stays.
  - **Tests changed.** Three tests asserted that the second request had no tools; they now assert it lists the same tools as the first, with choice `none` (hook, assistant, and the wire body in `local_llm_tests.rs`).
  - **Verified.** `scripts/verify.sh`: all checks passed (451 root unit tests).
  - **Not verified.** Jarvis itself on the Pi: the two `cached_input` criteria above need a rebuild there and a run with tool calls (`bench_first_text.py` has no tool-call script yet). Whether Gemma, seeing the tools on the second turn with choice `none`, ever writes tool-call markup as text; if it does, `validate_turn` or the speech path will show it.
