# 12: Catalog panels

Status: needs-triage
Stories: US5
Blocked by: 11
Assignee: unassigned
Owns: none yet. Set when the ticket is unblocked.
Traces to: D10

**Waiting on:** Darren's decisions on the open questions below. This is later work and is not yet designed.

**What to build (provisional):** Jarvis builds a Panel from the Catalog (glossary: card, chart, list, gauge, camera tile). The llama.cpp server returns the Panel as JSON, constrained by a `json_schema`. The server runs 2 slots of 4,096 tokens (AGENTS, current work). The UI renders the parts and never runs code from the model.

Open questions for Darren:
- The schema for each part, and the Catalog list.
- How the Panel JSON reaches the UI: over the WebSocket, as one ToOrb message.
- The limit on parts per Panel. Proposal P5, approved 2026-11-15: 8, as a starting point to revisit in this ticket.
- What happens when the output does not match the schema: nothing is shown, and Darren gets a Notice.
- The code-generated experiment (cloud model) is separate. It needs an explicit opt-in that covers everything sent (D10).

## Acceptance criteria (provisional, to be confirmed when unblocked)
- [ ] A Panel with 0 parts is rejected. A Panel with 1 part is accepted.
- [ ] A Panel with 8 parts is accepted. A Panel with 9 parts is rejected (P5, approved 2026-11-15).
- [ ] A part that is not in the Catalog is rejected.
- [ ] The longest Panel the schema allows fits in one 4,096-token slot. Stated with the count.
- [ ] Output that fails the schema shows no Panel and produces a Notice. Tested.
- [ ] No cloud request is made without the opt-in in D10.

## Comments
