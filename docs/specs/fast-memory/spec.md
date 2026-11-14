# Fast memory

Status: active
Resume here: 01 (search only on demand) is next; nothing is built yet.

Settled with Darren in a grilling session on 2026-11-14. This file is the source of truth for the epic. Tickets are in `issues/`; their `Status:` lines are the progress record.

## Problem Statement

Jarvis answers slowly. Measured on the Pi on 2026-11-14 with five typed questions against a copy of Darren's memory database: the wait for the first word totalled 32.9 s, against 3.1 s with an empty memory. Recall runs on every question and hands Gemma up to 5 memories (4,096 bytes). Recalled text is new each time, so it is never in Gemma's cache, and Gemma reads about 58 tokens a second on the 16 GB Pi: 341 recalled tokens cost 6 s on "Hello", 835 cost 15.6 s on "What time is it?". A keyword match skips the relevance floor, so a greeting recalls old greetings. Darren wants Jarvis to answer fast and still remember things about him.

## User Stories

1. As Darren, I want ordinary questions answered without a memory search, so that Jarvis starts talking in well under a second of model time.
2. As Darren, I want Jarvis to search our old conversations when I ask it to remember ("do you remember…", "last time…", `/recall …`), so that it can still use what we said before.
3. As Darren, I want Jarvis to say a short line at once when it starts such a search, so that the wait does not feel like silence.
4. As Darren, I want Jarvis to always know a short list of facts about me without searching, so that "what's my sister's name?" works with no trigger word and no delay.
5. As Darren, I want to add a fact by saying "remember that…", so that I decide what it keeps.
6. As Darren, I want Jarvis to note lasting facts about me by itself while it is idle, and show me what it noted, so that I do not have to dictate everything.
7. As Darren, I want to list facts and remove wrong ones by typing or by voice, so that the list stays true.

## Implementation Decisions

| ID | Decision |
| --- | --- |
| D1 | **No recall by default.** A prompt reaches Gemma with the system prompt, the facts list (D5) and the history only. |
| D2 | **Search on demand.** A search of stored memories runs for `/recall <question>` and for a prompt that contains a trigger phrase and is not a save (D6): "remember", "recall", "last time", "what did I tell you", "what did I say", "did I tell you". Matching is on whole words, case-insensitive. The decision is one pure function, deterministic, with no model involved. |
| D3 | **A triggered search is small.** At most 2 memories and 1,024 bytes (today 5 and 4,096). Measured 2026-11-14: this size costs up to about 3 s of reading. The limits are named constants, to be raised if searches miss things. The relevance rules are otherwise unchanged. |
| D4 | **Filler line.** On every triggered search Jarvis at once prints and speaks one of three canned lines in rotation ("Let me check.", "One moment.", "Let me look that up."), before the model run produces anything. No model writes it. If the search finds nothing, Gemma just answers. |
| D5 | **Facts list.** A short list of facts about Darren, sent right after the system prompt on every request so it stays in Gemma's prompt cache; it costs reading time only when it changes. Capped at about 300 tokens (a byte cap in code, about 1,200 bytes), each fact short. Facts are their own kind of record, apart from the searchable conversation memories. |
| D6 | **Saving a fact on request.** A line that starts with "remember that" (or `/remember <fact>`) saves the rest as a fact, in Darren's words as said or transcribed, with no model pass. Jarvis replies "Got it." (printed and spoken). This is a save, never a search. One exception keeps Darren's own wording working: "remember that time…", "remember that day…", "remember that night…" and "remember that one…" ask about the past, so they are searches (D2), not saves. A leading "hey Jarvis" changes nothing. |
| D7 | **Noting facts by itself.** While idle, Jarvis runs a Gemma pass over turns not yet examined and adds lasting facts Darren stated about himself: names, people, preferences, routines, places. Never anything from Jarvis's own answers, and nothing from a dropped or cancelled turn. User input always wins: a new prompt or tap drops the pass, and the turn is examined later. Each noted fact is shown as a terminal line ("Noted: …"), not spoken. |
| D8 | **When the list is full.** Facts Jarvis noted by itself are dropped oldest-first to make room. Facts Darren gave with "remember that" are never dropped automatically; if they alone fill the list, the new one is refused and Jarvis says so. |
| D9 | **Seeing and removing facts.** `/memory facts` lists them with numbers. `/memory forget <number>`, "forget that" right after a "Noted:" or "Got it." line, and "forget <detail>" remove one. These work typed or spoken, through the existing command parser. No model-called tool. |
| D10 | **Storing is unchanged.** Every completed turn is still archived and indexed as searchable memory. Only when memory is read changes. |
| D11 | **Delivery order.** First the speed fix (01, 02), tried on the Pi. Then the facts list (03), saving on request (04), noting by itself (05), listing and removing (06). Between the two steps Jarvis is fast but remembers only on a trigger word. |

## Testing Decisions

- Follow the AGENTS Testing section. The trigger decision is tested once as a table in the layer that owns it: each phrase, mixed case, a save that must not search, and a phrase inside a longer word ("remembering" does not match "remember").
- Limits at their boundaries: 2 vs 3 memories, 1,024 vs 1,025 bytes, the facts cap and one byte more.
- Model tests stream (`MockCompletionModel::from_stream_turns`). Assert the request: no memory message without a trigger, one with it; the facts list directly after the system prompt and byte-identical across two runs when unchanged.
- Local tests do not prove Pi speed. The Pi acceptance for 01 is the five-question measurement of 2026-11-14 repeated: ordinary questions near the empty-memory figures.

## Out of Scope

- A recall or forget tool that Gemma calls (kept as ticket 07, `needs-triage`).
- Pruning stored chit-chat, or changing what is archived.
- Changing the Gemma server's settings. Noted for later: it runs 4 slots on a 2,048-token context; one slot (`--parallel 1`) would give the conversation the whole context and one cache. Not measured as a cause of slowness.

## Further Notes

- Evidence for the Problem Statement and for D3 is in ticket 01's Comments once recorded: the baseline, the empty-memory run and three trial builds (2/1,024 bytes: 9.7 s; the same with keyword matches held to the relevance floor: 9.1 s; 1/600 bytes: 8.6 s).
- A recall inserts its message before the last exchange (real-time-speech R7b), so the turn after a search re-reads that exchange. With recall rare this cost is rare too.
- Whisper returned likely hallucinations on near-silent recordings in Darren's run ("Thanks for watching!"). Separate problem; not part of this epic.
