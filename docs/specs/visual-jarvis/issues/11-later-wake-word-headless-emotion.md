# 11: Later: wake word, headless Jarvis, emotion colour

Status: needs-triage
Stories: US8, US9, US10
Blocked by: 10
Assignee: unassigned
Owns: None until triaged

**What to build:** Three later stories, each to be split into its own spike or ticket after triage:

- **Wake word (US8).** Say "Hey Jarvis" to start listening. It reuses the VAD and `Listener` path. First a spike: verify that openWakeWord's "hey jarvis" model exists and works, and survey Rust-native detectors. The detector runs on the 8 GB Pi. Audio is never stored (push-to-talk D7). Any crate needs Darren's approval.
- **Headless Jarvis (US9).** Run Jarvis without a terminal, probably as a systemd user service, so the orb is the whole device experience. Needs a way to see errors (journal) and to start and stop it.
- **Emotion colour (US10).** Blue calm, gold engaged, red angry. A source is not designed. A probable route is Gemma tagging its own replies, sent as a separate emotion field in `orb-protocol`, not mixed into the state enum. Needs a design and a grilling session first.

## Acceptance criteria
- [ ] Each story is triaged, designed and split into tickets, or marked `wontfix`.

## Comments
