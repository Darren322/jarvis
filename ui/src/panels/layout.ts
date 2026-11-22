/**
 * Where the Orb and a Panel go on the screen. Pure: no DOM and no React.
 *
 * With no Panel the Orb fills the window. While a Panel shows (jarvis-ui D8)
 * the Orb shrinks into the top-left corner, where it keeps showing state and
 * taking taps, and the Panel takes the rest of the screen, inset from the
 * edges so there is room around it to tap outside and close it.
 *
 * The timer strip (timers D15) sits at the top right, Panel or not, in a
 * column that mirrors the corner Orb's: the same inset and width. While it has
 * rows and a Panel shows, the Panel's right edge moves left of that column, so
 * the strip never covers the Panel's frame and the Orb's corner stays clear.
 */

import type { PanelName } from '../link/protocol';
import type { OrbPosition, OrbSize } from '../orb/Orb';

/** The side of the Orb's square while a Panel shows, in CSS pixels: a third of the kiosk's 480 px height. */
export const CORNER_ORB_SIDE = 160;

/** The gap between the corner Orb's square and the top and left edges of the window, in CSS pixels. */
export const CORNER_ORB_INSET = 12;

/** The gap between the Panel and the window's top, right and bottom edges, in CSS pixels. */
export const PANEL_INSET = 20;

/** The gap between the corner Orb's square and the Panel's left edge, in CSS pixels. */
export const ORB_PANEL_GAP = 12;

/** The Panel's left edge, in CSS pixels: just right of the corner Orb. */
export const PANEL_LEFT = CORNER_ORB_INSET + CORNER_ORB_SIDE + ORB_PANEL_GAP;

export interface OrbPlacement {
  readonly size: OrbSize;
  readonly position: OrbPosition;
  /** The corner Orb is clipped to a circle: its square would cut the outer rings straight. */
  readonly round: boolean;
}

/** Where the Panel sits, as distances from the window's edges in CSS pixels. */
export interface PanelInsets {
  readonly left: number;
  readonly top: number;
  readonly right: number;
  readonly bottom: number;
}

/** The timer strip's width, in CSS pixels: the corner Orb's, so the screen reads as Orb, Panel, timers. */
export const TIMER_STRIP_WIDTH = CORNER_ORB_SIDE;

/** The gap between the timer strip and the window's top and right edges, in CSS pixels: the corner Orb's. */
export const TIMER_STRIP_INSET = CORNER_ORB_INSET;

/** The Panel's right inset while the timer strip has rows: just left of the strip, with the Orb's gap. */
export const PANEL_RIGHT_BESIDE_TIMERS = TIMER_STRIP_INSET + TIMER_STRIP_WIDTH + ORB_PANEL_GAP;

/**
 * Where the timer strip sits, the same whether or not a Panel shows:
 * distances from the window's top and right edges, its width, and the
 * window-relative cap on its height (the inset kept above and below), as CSS.
 */
export const TIMER_STRIP_BOX = {
  top: TIMER_STRIP_INSET,
  right: TIMER_STRIP_INSET,
  width: TIMER_STRIP_WIDTH,
  maxHeight: `calc(100vh - ${2 * TIMER_STRIP_INSET}px)`,
} as const;

const ORIGIN: OrbPosition = { left: 0, top: 0 };
const CORNER_POSITION: OrbPosition = { left: CORNER_ORB_INSET, top: CORNER_ORB_INSET };
const CORNER_SIZE: OrbSize = { width: CORNER_ORB_SIDE, height: CORNER_ORB_SIDE };

/** The Panel's place on the screen. The same for every Panel today. */
export const PANEL_INSETS: PanelInsets = {
  left: PANEL_LEFT,
  top: PANEL_INSET,
  right: PANEL_INSET,
  bottom: PANEL_INSET,
};

/** The Panel's place while the timer strip has rows (`hasTimers`) or not. */
export function panelInsets(hasTimers: boolean): PanelInsets {
  return hasTimers ? { ...PANEL_INSETS, right: PANEL_RIGHT_BESIDE_TIMERS } : PANEL_INSETS;
}

/** The Orb's size and top-left corner for a window of `windowSize` with `panel` shown (`null` for none). */
export function orbPlacement(windowSize: OrbSize, panel: PanelName | null): OrbPlacement {
  if (panel === null) {
    return { size: windowSize, position: ORIGIN, round: false };
  }
  return { size: CORNER_SIZE, position: CORNER_POSITION, round: true };
}
