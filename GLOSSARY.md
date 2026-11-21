# Jarvis

A local-first voice assistant for one person, Darren, with a touchscreen that shows what it is doing and what he asks to see.

## Screen

**Orb**:
The animated figure that shows Jarvis's state (idle, listening, transcribing, thinking, speaking) and reacts to voice loudness.
_Avoid_: avatar, character, blob

**Panel**:
Anything Jarvis shows on the screen besides the orb, such as stats, a camera view or a timer.
_Avoid_: window, widget, popup, view

**Dashboard**:
A panel built in advance with a known layout (for example, system stats) that Jarvis brings up when Darren asks for it.
_Avoid_: stats page, screen

**Catalog**:
The fixed set of panel parts from which Jarvis puts together a new panel on request (the `build_panel` tool): a card (title and text), a list (title and items) and a gauge (one live figure of the stats Dashboard). A chart and a camera tile may join it later.
_Avoid_: component library, widget set
