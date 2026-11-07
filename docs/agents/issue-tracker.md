# Issue tracker: Local Markdown (docs/specs)

Epics, user stories and tickets live as committed markdown files under `docs/specs/`. The Tickets tab in Claude Code reads these files, so keep the header lines exactly in the shapes below.

## Hierarchy

| Level | Where | Identity |
| --- | --- | --- |
| Epic | `docs/specs/<epic-slug>/spec.md` | The directory slug; the title is the first `# ` heading |
| User story | Numbered list under `## User Stories` in `spec.md` | `US1`, `US2`, ... (its position in the list) |
| Ticket (task) | `docs/specs/<epic-slug>/issues/<NN>-<slug>.md` | `NN`, numbered from `01` in dependency order |

Never put several tickets in one file.

## Epic file (`spec.md`)

```markdown
# <Epic title>

Status: active | done | parked
Resume here: <one line: what the next session does first>

## Problem Statement
...
## User Stories
1. As Darren, I want ..., so that ...
2. ...
```

The rest follows the `/to-spec` template (Solution, Implementation Decisions, Testing Decisions, Out of Scope, Further Notes).

## Ticket file

```markdown
# 03: <Ticket title>

Status: ready-for-agent
Stories: US1, US4
Blocked by: 01, 02
Assignee: agent:<name> | Darren | Claude | unassigned
Owns: src/features/listening/**, tests/unit/features/listening/**

**What to build:** ...

## Acceptance criteria
- [ ] ...

## Comments
```

- `Status:` one of `needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `in-progress`, `in-review`, `done`, `wontfix`. The ticket file is the source of truth for progress; no separate Status table in new-style specs.
- `Stories:` the user stories this ticket serves. Every story should be served by at least one ticket.
- `Blocked by:` ticket numbers, or `None`. A ticket is on the frontier when every blocker is `done`.
- `Assignee:` who is working on it now. Set it when a ticket is claimed; clear it to `unassigned` when work stops.
- `Owns:` the files and directories the ticket may change. This overrides the skills' "no file paths" rule: sub-agents need explicit, non-overlapping ownership (AGENTS.md).
- Acceptance criteria are checkboxes; tick them as they are verified.
- Comments append at the bottom under `## Comments`, newest last, each starting with a date.
- Specs written before this convention (`push-to-talk.md`, `real-time-speech.md`) stay single files with their Status tables; don't migrate them.

## When a skill says "publish to the issue tracker"

Create the file under `docs/specs/<epic-slug>/` (creating the directory if needed).

## When a skill says "fetch the relevant ticket"

Read the file at the referenced path. Darren will normally pass the path or the ticket number.

## Implementing (`/implement-spec`, `/implement`)

- The integration branch is `dev`. Never branch from, merge into, or open PRs against `main`.
- Implementer worktrees start from `dev` (check `git log --oneline -1`; `git reset --hard dev` if not) and build with `CARGO_TARGET_DIR=<main checkout>/target`.
- Implementers set `Status: in-progress` and their `Assignee:` when they start, and `Status: in-review` when the AGENTS.md Verification list passes.
- **Implementers never commit.** They leave changes uncommitted in their worktree. Claude reviews and integrates the change into the `dev` working tree uncommitted; Darren reviews and commits it, then the ticket becomes `done`.

## Wayfinding operations

Used by `/wayfinder`. The **map** is a file with one **child** file per ticket.

- **Map**: `docs/specs/<effort>/map.md` (the Notes / Decisions-so-far / Fog body).
- **Child ticket**: `docs/specs/<effort>/issues/NN-<slug>.md`, numbered from `01`, with the question in the body. A `Type:` line records the ticket type (`research`/`prototype`/`grilling`/`task`); a `Status:` line records `claimed`/`resolved`.
- **Blocking**: a `Blocked by: NN, NN` line near the top. A ticket is unblocked when every file it lists is `resolved`.
- **Frontier**: scan `docs/specs/<effort>/issues/` for files that are open, unblocked, and unclaimed; first by number wins.
- **Claim**: set `Status: claimed` and save before any work.
- **Resolve**: append the answer under an `## Answer` heading, set `Status: resolved`, then append a context pointer (gist + link) to the map's Decisions-so-far in `map.md`.
