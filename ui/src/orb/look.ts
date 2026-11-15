/**
 * What the Orb should look like for a given view and loudness.
 *
 * Plain data and pure functions only: no drawing and no clock. The renderer turns
 * a `Look` into pixels, and the animator eases from one `Look` to the next with
 * `lerpLookInto`, so a change of state never snaps.
 *
 * Ported from the approved macroquad orb (`crates/jarvis-orb/src/look.rs`,
 * visual-jarvis D1 to D3 and ticket 12). Keep the two in step until that crate
 * is retired (jarvis-ui ticket 06).
 */

import type { OrbState } from '../link/protocol';

/** A colour as red, green, blue, each 0 to 1. */
export type Rgb = [number, number, number];

/** What the Orb shows: one of Jarvis's states, or the offline look. */
export type View = 'offline' | OrbState;

/** Every view, offline first. */
export const VIEWS: readonly View[] = [
  'offline',
  'idle',
  'listening',
  'transcribing',
  'thinking',
  'speaking',
];

/**
 * How sharply the level curve rises: the response is `1 - exp(-LEVEL_GAIN * level)`.
 * Speech RMS is small (about 0.01 to 0.1), so a gain of 18 makes normal speech
 * move the Orb visibly without saturating it.
 */
export const LEVEL_GAIN = 18;

/**
 * How fast one look eases into the next, per second. About 95% of a change is
 * done after `3 / EASE_RATE`, a third of a second.
 */
export const EASE_RATE = 9;

/** Everything the drawing needs. Every field is a number or a colour, so two looks blend field by field. */
export interface Look {
  /** Colour of the starburst, the core ring and the rays. */
  coreColor: Rgb;
  /** Colour of the translucent fill inside the core ring and behind the glyph band. */
  fillColor: Rgb;
  /** Colour of the band of glyphs around the core. */
  bandColor: Rgb;
  /** Colour of the HUD rings and ticks. */
  hudColor: Rgb;
  /** Overall brightness, 0 (invisible) to 1. */
  brightness: number;
  /** Turn of the sparkle cloud inside the core ring, radians per second. */
  sparkleSpin: number;
  /** Turn of the glyph band, radians per second. */
  bandSpin: number;
  /** Turn of the HUD rings, radians per second; each ring turns at a fixed multiple of it. */
  hudSpin: number;
  /** How visible the glyph band is, 0 to 1. */
  bandAlpha: number;
  /** How visible the HUD rings and ticks are, 0 to 1. */
  hudAlpha: number;
  /** How far the Orb swells and shrinks as it breathes, as a fraction of its radius. */
  breathDepth: number;
  /** Breaths per second. */
  breathRate: number;
  /** How strongly the core ring and the sparkles swell in a rippling pulse, 0 to 1. */
  nodePulse: number;
  /** How far the sparkles move in and out, 0 to 1. The outermost HUD ring flares with it. */
  nodeJitter: number;
  /** Size and brightness of the core, 0 to 1. */
  coreGlow: number;
  /** Length and brightness of the light rays, 0 (none) to 1. */
  rayStrength: number;
  /** How visible the drifting background blobs are, 0 to 1. */
  background: number;
  /** How coloured the background blobs are: 0 is grey, 1 is full colour. */
  blobSaturation: number;
}

/** The colour fields of a `Look`. */
export const COLOR_KEYS = ['coreColor', 'fillColor', 'bandColor', 'hudColor'] as const;

/** The number fields of a `Look`. */
export const NUMBER_KEYS = [
  'brightness',
  'sparkleSpin',
  'bandSpin',
  'hudSpin',
  'bandAlpha',
  'hudAlpha',
  'breathDepth',
  'breathRate',
  'nodePulse',
  'nodeJitter',
  'coreGlow',
  'rayStrength',
  'background',
  'blobSaturation',
] as const;

type NumberKey = (typeof NUMBER_KEYS)[number];

/** A look at silence, plus how much each number field gains at a full level response. */
interface LookRule {
  readonly base: Readonly<Look>;
  readonly perLevel: Readonly<Partial<Record<NumberKey, number>>>;
}

/**
 * Per-view looks. Offline is neutral grey; Idle is a cool blue HUD with a soft
 * amber core; Thinking is the full gold look. Only Listening (the microphone)
 * and Speaking (Jarvis's voice) use the level.
 */
const RULES: Readonly<Record<View, LookRule>> = {
  offline: {
    base: {
      coreColor: [0.55, 0.55, 0.55],
      fillColor: [0.4, 0.4, 0.4],
      bandColor: [0.5, 0.5, 0.5],
      hudColor: [0.45, 0.45, 0.45],
      brightness: 0.3,
      sparkleSpin: 0.06,
      bandSpin: 0.04,
      hudSpin: -0.03,
      bandAlpha: 0.25,
      hudAlpha: 0.3,
      breathDepth: 0.035,
      breathRate: 0.2,
      nodePulse: 0,
      nodeJitter: 0,
      coreGlow: 0.18,
      rayStrength: 0,
      background: 0.3,
      blobSaturation: 0,
    },
    perLevel: {},
  },
  idle: {
    base: {
      coreColor: [1.0, 0.62, 0.3],
      fillColor: [0.85, 0.45, 0.1],
      bandColor: [0.8, 0.52, 0.22],
      hudColor: [0.15, 0.55, 0.95],
      brightness: 0.6,
      sparkleSpin: 0.14,
      bandSpin: 0.1,
      hudSpin: -0.07,
      bandAlpha: 0.35,
      hudAlpha: 0.85,
      breathDepth: 0.045,
      breathRate: 0.25,
      nodePulse: 0,
      nodeJitter: 0,
      coreGlow: 0.35,
      rayStrength: 0.1,
      background: 0.5,
      blobSaturation: 0.6,
    },
    perLevel: {},
  },
  listening: {
    base: {
      coreColor: [1.0, 0.72, 0.4],
      fillColor: [0.9, 0.55, 0.15],
      bandColor: [0.95, 0.7, 0.3],
      hudColor: [0.25, 0.85, 1.0],
      brightness: 0.7,
      sparkleSpin: 0.18,
      bandSpin: 0.12,
      hudSpin: -0.08,
      bandAlpha: 0.55,
      hudAlpha: 0.7,
      breathDepth: 0.03,
      breathRate: 0.35,
      nodePulse: 0,
      nodeJitter: 0,
      coreGlow: 0.4,
      rayStrength: 0.1,
      background: 0.6,
      blobSaturation: 0.8,
    },
    perLevel: { brightness: 0.3, hudAlpha: 0.3, nodePulse: 1, coreGlow: 0.4 },
  },
  transcribing: {
    base: {
      coreColor: [1.0, 0.7, 0.38],
      fillColor: [0.95, 0.5, 0.1],
      bandColor: [1.0, 0.68, 0.22],
      hudColor: [0.2, 0.65, 1.0],
      brightness: 0.75,
      sparkleSpin: 0.16,
      // The glyph band spins fast: the "working on your words" cue.
      bandSpin: 2.4,
      hudSpin: -0.3,
      bandAlpha: 0.9,
      hudAlpha: 0.6,
      breathDepth: 0.03,
      breathRate: 0.5,
      nodePulse: 0,
      nodeJitter: 0,
      coreGlow: 0.45,
      rayStrength: 0.15,
      background: 0.6,
      blobSaturation: 0.9,
    },
    perLevel: {},
  },
  thinking: {
    base: {
      coreColor: [1.0, 0.78, 0.22],
      fillColor: [1.0, 0.55, 0.08],
      bandColor: [1.0, 0.6, 0.15],
      hudColor: [0.25, 0.9, 1.0],
      brightness: 0.85,
      sparkleSpin: 0.22,
      // Opposite directions: the glyph band and the HUD rings grind against each other.
      bandSpin: 1.0,
      hudSpin: -0.8,
      bandAlpha: 0.9,
      hudAlpha: 0.95,
      breathDepth: 0.05,
      breathRate: 0.8,
      nodePulse: 0,
      nodeJitter: 0,
      coreGlow: 0.95,
      rayStrength: 0.55,
      background: 0.8,
      blobSaturation: 1,
    },
    perLevel: {},
  },
  speaking: {
    base: {
      coreColor: [1.0, 0.85, 0.55],
      fillColor: [1.0, 0.6, 0.15],
      bandColor: [0.95, 0.72, 0.3],
      hudColor: [0.25, 0.8, 1.0],
      brightness: 0.75,
      sparkleSpin: 0.2,
      bandSpin: 0.2,
      hudSpin: -0.15,
      bandAlpha: 0.65,
      hudAlpha: 0.65,
      breathDepth: 0.04,
      breathRate: 0.5,
      nodePulse: 0,
      nodeJitter: 0,
      coreGlow: 0.5,
      rayStrength: 0.15,
      background: 0.7,
      blobSaturation: 1,
    },
    perLevel: { brightness: 0.25, nodePulse: 0.35, nodeJitter: 1, coreGlow: 0.5, rayStrength: 0.85 },
  },
};

/** Pulls a raw level into 0 to 1: below 0 becomes 0, above 1 becomes 1, NaN becomes 0. */
export function clampLevel(level: number): number {
  if (Number.isNaN(level)) {
    return 0;
  }
  return Math.min(1, Math.max(0, level));
}

/** Maps a raw level to a lively 0 to 1 response; 0 at 0, rising steeply, nearly 1 at 1. */
export function levelResponse(level: number): number {
  return 1 - Math.exp(-LEVEL_GAIN * clampLevel(level));
}

/** The fraction of the way to the target to move after `dt` seconds. A negative or NaN `dt` moves nothing. */
export function easeFactor(dt: number): number {
  if (Number.isNaN(dt) || dt <= 0) {
    return 0;
  }
  return 1 - Math.exp(-EASE_RATE * dt);
}

/** A fresh look with every field zero, to be filled by `lookInto` or `lerpLookInto`. */
export function emptyLook(): Look {
  return {
    coreColor: [0, 0, 0],
    fillColor: [0, 0, 0],
    bandColor: [0, 0, 0],
    hudColor: [0, 0, 0],
    brightness: 0,
    sparkleSpin: 0,
    bandSpin: 0,
    hudSpin: 0,
    bandAlpha: 0,
    hudAlpha: 0,
    breathDepth: 0,
    breathRate: 0,
    nodePulse: 0,
    nodeJitter: 0,
    coreGlow: 0,
    rayStrength: 0,
    background: 0,
    blobSaturation: 0,
  };
}

/**
 * Writes the look for `view` at a raw `level` into `out` and returns it. The
 * level is clamped to 0 to 1 first. Allocates nothing, so the frame loop can
 * call it every frame.
 */
export function lookInto(out: Look, view: View, level: number): Look {
  const rule = RULES[view];
  const response = levelResponse(level);
  for (const key of COLOR_KEYS) {
    const from = rule.base[key];
    const to = out[key];
    to[0] = from[0];
    to[1] = from[1];
    to[2] = from[2];
  }
  for (const key of NUMBER_KEYS) {
    out[key] = rule.base[key] + (rule.perLevel[key] ?? 0) * response;
  }
  return out;
}

/** The look for `view` at a raw `level`. Pure: the same input always gives an equal, new look. */
export function look(view: View, level: number): Look {
  return lookInto(emptyLook(), view, level);
}

/**
 * Exact at both ends (t of 1 gives `to` with no rounding left over) and still
 * when `from` equals `to`, so a settled look stays bit-for-bit equal to its target.
 */
function mix(from: number, to: number, t: number): number {
  return t >= 1 ? to : from + (to - from) * t;
}

/**
 * Blends `from` towards `to` into `out` (which may be `from`): `t` of 0 gives
 * `from` and 1 gives `to`. `t` is clamped to 0 to 1 (NaN counts as 0).
 */
export function lerpLookInto(out: Look, from: Look, to: Look, t: number): Look {
  const amount = clampLevel(t);
  for (const key of COLOR_KEYS) {
    const a = from[key];
    const b = to[key];
    const o = out[key];
    o[0] = mix(a[0], b[0], amount);
    o[1] = mix(a[1], b[1], amount);
    o[2] = mix(a[2], b[2], amount);
  }
  for (const key of NUMBER_KEYS) {
    out[key] = mix(from[key], to[key], amount);
  }
  return out;
}
