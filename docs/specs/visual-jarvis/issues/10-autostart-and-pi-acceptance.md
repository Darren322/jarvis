# 10: Autostart, full screen and Pi acceptance

Status: ready-for-human
Stories: US1, US2, US3, US4, US5, US6, US7
Blocked by: 09
Assignee: unassigned
Owns: scripts/orb/**, docs/ORB_DEPLOYMENT.md

**What to build:** Put `jarvis-orb` on the 8 GB Pi and prove the whole path. The agent prepares an aarch64 build, a labwc autostart entry (`~/.config/labwc/autostart`, which does not exist yet) that starts `jarvis-orb` at desktop login, and a short record of what was installed in `docs/ORB_DEPLOYMENT.md`. Every change on the Pi needs Darren's OK first (push-to-talk D10); the agent says exactly what it will change, then does it. Jarvis is started by hand with `run.sh` in a terminal (D11). The speech-to-text settings from push-to-talk must already be in the Pi's `.env`.

## Acceptance criteria
- [ ] Darren's OK for each Pi change is recorded in Comments.
- [ ] After login the orb starts full screen with the cursor hidden; with Jarvis stopped it shows the offline look and ignores taps; starting Jarvis connects it within a few seconds.
- [ ] Darren taps while idle, speaks, stops talking, and hears the answer, with no key pressed.
- [ ] The orb shows every state: Idle, Listening (reacts to his voice), Transcribing, Thinking and Speaking (reacts to the voice).
- [ ] A tap while speaking stops speech and listens; a tap while thinking drops that answer and listens; a tap while listening sends at once; a tap while transcribing does nothing.
- [ ] Frame rate and CPU while Jarvis speaks are noted; a reboot brings the orb back.
- [ ] The way to remove the autostart entry is documented. The Status of tickets 01 to 09 is checked and the spec's Resume line updated.

## Comments
