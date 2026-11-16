/**
 * The Panels' colours, taken from the Orb's looks (`orb/look.ts`) so a Panel
 * and the Orb read as one HUD: the idle blue for lines, the listening cyan for
 * readings, and the idle amber core for a service that is down.
 */

import type { CSSProperties } from 'react';
import { look, type Rgb } from '../orb/look';

/** A look colour (0 to 1 per channel) as a CSS `rgb()` with an alpha of 0 to 1. */
export function cssColor([red, green, blue]: Rgb, alpha = 1): string {
  const channel = (value: number) => Math.round(value * 255);
  return `rgb(${channel(red)} ${channel(green)} ${channel(blue)} / ${alpha})`;
}

const IDLE = look('idle', 0);
const LISTENING = look('listening', 0);

/**
 * The palette as CSS custom properties, set once on each Panel's root and read
 * by `panels.css`.
 */
export const PANEL_COLORS = {
  /** The Panel's background: the idle HUD blue, barely there over the black. */
  '--panel-fill': cssColor(IDLE.hudColor, 0.04),
  /** Frame, rules and meter tracks: the idle HUD blue, faint. */
  '--panel-line': cssColor(IDLE.hudColor, 0.38),
  '--panel-track': cssColor(IDLE.hudColor, 0.22),
  /** Corner brackets and labels: the idle HUD blue, brighter. */
  '--panel-accent': cssColor(IDLE.hudColor, 0.9),
  '--panel-label': cssColor(LISTENING.hudColor, 0.72),
  /** Meter fill and a service that is up: the listening cyan. */
  '--panel-reading': cssColor(LISTENING.hudColor),
  /** A service that is down: the idle core's amber. */
  '--panel-alert': cssColor(IDLE.coreColor),
  /** Values: near white, tinted towards the cyan. */
  '--panel-ink': 'rgb(236 248 255)',
  '--panel-ink-dim': 'rgb(236 248 255 / 0.6)',
} as const satisfies Record<`--${string}`, string>;

/** `PANEL_COLORS` as a React style. */
export const PANEL_COLOR_STYLE = PANEL_COLORS as CSSProperties;
