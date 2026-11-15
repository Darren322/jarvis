/**
 * Which thinking-orbs motion the Orb's body shows for each view, and how the
 * level drives it.
 *
 * The motion comes from the `thinking-orbs` package (MIT, Jakub Antalik,
 * https://libraries.dev/orbs), driven through its engine at our own size.
 * Its options are the knobs its mode functions read (`latRings`, `rBase`,
 * `wobMul`, ...); a knob left out takes the mode's own default, which was
 * tuned for a 300 px frame.
 */

import type { ModeKey, ModeOpts } from 'thinking-orbs/engine';
import { levelResponse, type View } from '../look';

/** The body frame size the engine's defaults were tuned for, in CSS pixels. */
export const ENGINE_REFERENCE_SIZE = 300;

/**
 * Count knobs scaled by the square root of the count scale: grid pairs (their
 * product follows the count scale), and the plexus nodes, which need a
 * density floor or the links vanish on a small Orb.
 */
const SQRT_KNOBS = ['latRings', 'lonDensity', 'rings', 'lanes', 'segs', 'nodeN'] as const;
/** Count knobs that stand alone; each is scaled by the count scale. */
const COUNT_KNOBS = ['orbitN', 'ghostN', 'signals', 'strandN'] as const;
/** Radius knobs, scaled by the level pulse. */
const RADIUS_KNOBS = ['rBase', 'rDepth', 'nodeR', 'nodeRDepth', 'partR', 'partRDepth', 'ghostR'] as const;

/**
 * Every dot radius is this many times the engine's own. Its radii were tuned
 * for a small monochrome spinner; ours sits behind HUD rings and a glow, so it
 * needs heavier dots to read.
 */
const DOT_SCALE = 1.4;

/** How one view drives the engine. */
interface BodyRule {
  readonly mode: ModeKey;
  /** Engine seconds per second at silence. */
  readonly speed: number;
  /** Extra engine seconds per second at a full level response. */
  readonly speedPerLevel: number;
  /** How much the dot radii grow at a full level response (0 is none). */
  readonly pulsePerLevel: number;
  /** Ribbon undulation at silence and its gain at a full level response (ribbon modes only). */
  readonly wobble?: readonly [number, number];
  /** Options at full size, before count scaling. */
  readonly opts: Readonly<ModeOpts>;
}

/**
 * The body per view. Dot counts are kept near 300 (cost on a 2D canvas grows
 * with every dot): offline and transcribing are a dotted globe (transcribing
 * with the scan meridian), idle is the plexus web, listening is the latitude
 * wave, thinking is particles on tilted orbits, speaking is the undulating sash.
 */
const RULES: Readonly<Record<View, BodyRule>> = {
  offline: {
    mode: 'globe',
    speed: 0.35,
    speedPerLevel: 0,
    pulsePerLevel: 0,
    opts: { latRings: 13, lonDensity: 30, rBase: 0.7, rDepth: 1.4, rBoost: 0, dimBase: 1 },
  },
  idle: {
    mode: 'web',
    speed: 0.7,
    speedPerLevel: 0,
    pulsePerLevel: 0,
    opts: { nodeN: 44, thr: 0.62, signals: 4, nodeR: 1.3, nodeRDepth: 1.9, lineW: 0.9 },
  },
  listening: {
    mode: 'wave',
    speed: 0.7,
    speedPerLevel: 1.6,
    pulsePerLevel: 0.6,
    opts: { rings: 14, lonDensity: 32, rBase: 0.55, rDepth: 1.3 },
  },
  transcribing: {
    mode: 'globe',
    speed: 1.0,
    speedPerLevel: 0,
    pulsePerLevel: 0,
    opts: { latRings: 15, lonDensity: 34, rBase: 0.7, rDepth: 1.7, rBoost: 2.6, scanMul: 1.6, dimBase: 0.3 },
  },
  thinking: {
    mode: 'braid',
    speed: 1.6,
    speedPerLevel: 0,
    pulsePerLevel: 0,
    opts: { strandN: 96, turns: 2, ghostN: 70, rBase: 0.8, rDepth: 1.3 },
  },
  speaking: {
    mode: 'ring',
    speed: 0.9,
    speedPerLevel: 1.2,
    pulsePerLevel: 0.25,
    wobble: [0.35, 0.8],
    opts: { lanes: 5, segs: 60, ghostN: 70, faceOn: 1, rBase: 0.8, rDepth: 1.3, spin: 0.6 },
  },
};

/** Each rule's option names, listed once so a frame does not allocate them. */
const OPT_KEYS: Readonly<Record<View, readonly string[]>> = {
  offline: Object.keys(RULES.offline.opts),
  idle: Object.keys(RULES.idle.opts),
  listening: Object.keys(RULES.listening.opts),
  transcribing: Object.keys(RULES.transcribing.opts),
  thinking: Object.keys(RULES.thinking.opts),
  speaking: Object.keys(RULES.speaking.opts),
};

/** The engine mode a view shows. */
export function bodyMode(view: View): ModeKey {
  return RULES[view].mode;
}

/** Engine seconds per second for `view` at a raw `level`. */
export function bodySpeed(view: View, level: number): number {
  const rule = RULES[view];
  return rule.speed + rule.speedPerLevel * levelResponse(level);
}

/**
 * How many of the full-size dots to draw at a body frame of `size` CSS pixels:
 * 1 at the reference size and above, down to 0.3 for a small corner Orb.
 */
export function countScale(size: number): number {
  return Math.min(1, Math.max(0.3, size / ENGINE_REFERENCE_SIZE));
}

/**
 * Writes the engine options for `view` at a raw `level` and a count scale into
 * `out`, which the caller keeps per view so no frame allocates.
 */
export function writeBodyOpts(out: ModeOpts, view: View, level: number, scale: number): ModeOpts {
  const rule = RULES[view];
  const response = levelResponse(level);
  for (const key of OPT_KEYS[view]) {
    out[key] = rule.opts[key];
  }
  for (const key of SQRT_KNOBS) {
    const value = rule.opts[key];
    if (value !== undefined) {
      out[key] = Math.max(2, Math.round(value * Math.sqrt(scale)));
    }
  }
  for (const key of COUNT_KNOBS) {
    const value = rule.opts[key];
    if (value !== undefined && value > 0) {
      out[key] = Math.max(1, Math.round(value * scale));
    }
  }
  const pulse = DOT_SCALE * (1 + rule.pulsePerLevel * response);
  for (const key of RADIUS_KNOBS) {
    const value = rule.opts[key];
    if (value !== undefined) {
      out[key] = value * pulse;
    }
  }
  if (rule.wobble !== undefined) {
    out.wobMul = rule.wobble[0] + rule.wobble[1] * response;
  }
  return out;
}
