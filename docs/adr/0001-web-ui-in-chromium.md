---
Status: accepted (Darren, 2026-11-15)
---

# The touchscreen UI is a React page in Chromium kiosk, and the Orb draws with WebGL

## Context

Visual-jarvis D14 chose a native macroquad program (`jarvis-orb`) over a web page in Chromium. That was enough for one animated Orb. Darren now wants Panels: a stats Dashboard on request, a camera view, and later Panels that Jarvis builds from a Catalog. macroquad has no layout, text or components, so each Panel would be drawn by hand, and npm components (such as the MIT "Thinking orbs" on libraries.dev, or catalog renderers like json-render) could not be reused.

## Decision

The UI is a React, TypeScript and Vite app in `ui/`, built on the Mac and opened from disk by Chromium in kiosk mode on the 8 GB Pi. Jarvis serves `orb-protocol` over a localhost WebSocket. The Orb draws with WebGL2, not a 2D canvas. This supersedes visual-jarvis D14.

## Measurements (8 GB Pi 5, Chromium 154, 800x480, 2026-11-15)

From `notes/ui-spike/measurements-2026-11-15.md` (untracked). Memory is the summed PSS of all Chromium processes; CPU is percent of one core over 30 s.

| Page | RAM | CPU | FPS |
| --- | --- | --- | --- |
| macroquad `jarvis-orb` (D14 baseline) | 43 MB | 5% | not shown |
| Chromium, blank page | 425 MB | 2% | 60 |
| Chromium, WebGL2 orb (400 points, ~4,100 links) | 453 MB | 25% | 60 |
| Chromium, 2D-canvas orb, default or GPU-raster flags | 533–542 MB | 142% | 45 |

Against the original budget (D11: 500 MB or less, 15% of one core or less idle, 60 fps), the WebGL orb passes RAM and frame rate and misses CPU. Darren raised the idle CPU target to about 25% of one core ("not a huge impact"). The 2D canvas fails all three, so the Orb must not use it. WebGL2 runs on the real V3D driver (ANGLE, OpenGL ES 3.1), not a software fallback.

## Considered options

- **Keep macroquad and draw Panels by hand.** About 10x cheaper, but every Panel and every text layout would be custom code, with no npm reuse.
- **Slint (native, can load UI at runtime).** Lighter than Chromium, but its Pi backend is experimental and it cannot reuse npm components. The fallback if Chromium's cost becomes a problem.
- **Native Orb, Chromium only while a Panel shows.** Saves memory when idle, but about 2 s to open and two programs fighting for the screen.
- **Tauri, WPE/Cog, flutter-pi.** Rejected: slow canvas/WebGL paths on WebKitGTK, no Pi 5 support listed, or console-only mode. See `notes/ui-research/01-runtimes.md`.

## Consequences

- Chromium costs about 425 MB even on a blank page. That is the price of the web stack.
- The repo gains a TypeScript toolchain (npm on the Mac only; no Node on the Pi).
- Update 2026-11-15: Darren then chose the MIT `thinking-orbs@0.3.2` package as the Orb body (2D canvas, driven through its engine at 400 px: 29 to 48% of one core at about 60 fps, against 25% for the WebGL test orb), accepting about 50% of one core; the D1 touches are a WebGL2 layer above it. This is a deliberate exception to "no 2D canvas": the package draws about 30 to 60 dots, not the 400-point plexus that cost 142%.
- The measured WebGL page had no glyph rings or light rays yet, and nobody watched it on the screen. The full Orb must be measured again (ticket 04), including a 30 fps idle mode.
