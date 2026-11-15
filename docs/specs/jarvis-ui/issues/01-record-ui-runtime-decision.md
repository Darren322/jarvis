# 01: Record the UI runtime decision

Status: done
Stories: US1
Blocked by: None
Assignee: coordinator
Owns: `docs/adr/`
Traces to: D1

**What to build:** Write the ADR `docs/adr/0001-web-ui-in-chromium.md` from the Pi measurement. It records why a web UI in Chromium replaces the macroquad Orb, and that this reverses visual-jarvis D14. It holds the measured numbers and the alternatives that were tried.

## Acceptance criteria
- [x] The measurement file exists and names the Pi, its OS, the Chromium version, and the RAM, CPU and frame rate of the Orb page.
- [x] The ADR has Context, Decision, Alternatives and Consequences sections.
- [x] The ADR checks the numbers against D11 (500 MB RAM or less, about 15% of one core or less while idle, 60 fps) and says which pass and which do not.
- [x] The ADR says visual-jarvis D14 is superseded.
- [x] Darren confirms the decision in Comments. Then D1 in the spec loses its PENDING mark.

## Comments

- 2026-11-15 (coordinator): Two runs on the 8 GB Pi (`notes/ui-spike/measurements-2026-11-15.md`, untracked). WebGL2 orb: 453 MB PSS, 25% of one core, 60 fps. 2D-canvas orb: about 540 MB, 142%, 45 fps, with or without GPU-raster flags. Blank Chromium page: 425 MB, 2%. Darren chose web with a WebGL2 orb and raised the idle CPU target to about 25% of one core ("Yes sure" to Q21). ADR 0001 written; spec D1, D5 and D11 updated.
