/**
 * The static geometry of the WebGL layer, built once: background blobs, light
 * rays, HUD rings, the glyph band and the starburst rays. Every record is an
 * instance for an instanced draw; the shaders animate them from a few
 * uniforms, so a frame uploads no vertex data.
 *
 * Ported from `Orb::new` and the constants in `crates/jarvis-orb/src/orb.rs`.
 * Distances and radii are fractions of the drawing area's shorter side;
 * widths are in "reference pixels", pixels on the 480 px tall screen.
 */

import { ARC_STEPS, GLYPHS, type Stroke } from './glyphs';

const TAU = Math.PI * 2;
const DEGREES = Math.PI / 180;

/**
 * Shader time wraps at this many seconds. Every rate the shaders use with it
 * is rounded to a whole number of turns per period (`loopRate`), so the wrap
 * is seamless and float precision never decays however long the Orb runs.
 */
export const TIME_PERIOD = 1200;

/** `rate` (radians per second) rounded to a whole number of turns per `TIME_PERIOD`. */
export function loopRate(rate: number): number {
  return (Math.round((rate * TIME_PERIOD) / TAU) * TAU) / TIME_PERIOD;
}

/** Seed for placement noise, so the Orb is the same on every start. */
const SEED = 0x0b1a57;

/** mulberry32: a small seeded generator, so placement is the same on every start. */
function seededRandom(seed: number): () => number {
  let state = seed >>> 0;
  return () => {
    state = (state + 0x6d2b79f5) >>> 0;
    let t = state;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

// Groups: each arc or stroke belongs to one, and the renderer sets each
// group's turn, scale, opacity and colour per frame.
/** HUD rings 0 to 6, inside out. */
export const HUD_GROUPS = 7;
export const GLYPH_GROUP = 7;
export const BACKING_GROUP = 8;
export const GROUP_COUNT = 9;

// Background blobs.
const BLOB_RING_RADII = [0.28, 0.5, 0.75, 1.0];
const BLOBS_PER_RING = [6, 9, 10, 11];
const BLOB_RADIUS_MIN = 0.04;
const BLOB_RADIUS_MAX = 0.11;
/** Fastest drift around the centre, radians per second. */
const BLOB_DRIFT_MAX = 0.05;
export const BLOB_WOBBLE_RATE = loopRate(0.3);
const BLOB_COLORS: readonly (readonly [number, number, number, number])[] = [
  // [cumulative chance, r, g, b]: blue, teal, green, amber, pink.
  [0.35, 0.1, 0.3, 0.95],
  [0.65, 0.1, 0.7, 0.75],
  [0.8, 0.3, 0.85, 0.4],
  [0.92, 1.0, 0.6, 0.15],
  [1.0, 1.0, 0.7, 0.8],
];

// Light rays.
const RAY_COUNT = 64;
/** Every this-many rays is a thick, bright one. */
const RAY_THICK_EVERY = 8;
const RAY_WIDTH_PX = 0.9;
const RAY_THICK_WIDTH_PX = 2.0;
const RAY_WHITE_MAX = 0.7;
export const RAY_SHIMMER_RATE = loopRate(2.2);

// Starburst rays.
const STAR_RAY_COUNT = 40;
const STAR_RAY_PX = 1.0;
const STAR_RAY_WHITE = 0.75;

// Glyph band.
export const BAND_RADIUS = 0.42;
const BAND_HEIGHT = 0.125;
const BAND_WIDTH = 0.034;
const BAND_GLYPHS = 66;
const GLYPH_LINE_PX = 1.7;
const GLYPH_STRIDE = 3;
export const GLYPH_SHIMMER_RATE = loopRate(1.3);

// HUD rings.
const HUD_GLOW_WIDTH = 4.0;
const HUD_GLOW_ALPHA = 0.16;
const HUD_NOTCH_PX = 5.0;
const HUD_NOTCH_ALPHA = 0.9;
const HUD_TICK_PX = 1.0;
const HUD_MAJOR_TICK_PX = 1.8;
const HUD_MAJOR_TICK_BOOST = 1.3;
/** Reference pixels per shorter side. */
const REFERENCE_SIZE = 480;

type HudKind =
  | { readonly kind: 'circle'; readonly thicknessPx: number }
  | { readonly kind: 'arcs'; readonly arcs: readonly (readonly [number, number])[]; readonly thicknessPx: number; readonly notches: boolean }
  | { readonly kind: 'dashes'; readonly count: number; readonly duty: number; readonly thicknessPx: number }
  | { readonly kind: 'ticks'; readonly count: number; readonly majorEvery: number; readonly minorLen: number; readonly majorLen: number };

/** One HUD ring. `spin` is a multiple of the look's HUD spin; `flare` marks the ring that flares with speech. */
export interface HudRing {
  readonly radius: number;
  readonly spin: number;
  readonly alpha: number;
  readonly flare: boolean;
  readonly shape: HudKind;
}

/** Inside out, as in the macroquad orb. */
export const HUD_RINGS: readonly HudRing[] = [
  { radius: 0.235, spin: 0.5, alpha: 0.8, flare: false, shape: { kind: 'ticks', count: 72, majorEvery: 6, minorLen: 0.008, majorLen: 0.016 } },
  {
    radius: 0.335,
    spin: 1.0,
    alpha: 0.85,
    flare: false,
    shape: { kind: 'arcs', arcs: [[0, 70], [80, 25], [115, 60], [185, 15], [210, 95], [315, 30]], thicknessPx: 1.6, notches: false },
  },
  { radius: 0.356, spin: 0, alpha: 0.7, flare: false, shape: { kind: 'circle', thicknessPx: 1.2 } },
  { radius: 0.484, spin: 0.3, alpha: 0.8, flare: false, shape: { kind: 'dashes', count: 72, duty: 0.5, thicknessPx: 1.4 } },
  {
    radius: 0.525,
    spin: -0.7,
    alpha: 0.85,
    flare: false,
    shape: { kind: 'arcs', arcs: [[10, 50], [70, 10], [90, 100], [205, 35], [250, 70], [330, 20]], thicknessPx: 3.0, notches: true },
  },
  { radius: 0.575, spin: 0.4, alpha: 0.8, flare: false, shape: { kind: 'ticks', count: 120, majorEvery: 5, minorLen: 0.01, majorLen: 0.022 } },
  {
    radius: 0.68,
    spin: 1.4,
    alpha: 0.9,
    flare: true,
    shape: { kind: 'arcs', arcs: [[0, 100], [115, 60], [190, 40], [250, 80], [340, 15]], thicknessPx: 5.0, notches: true },
  },
];

/** Floats per instance of each layer. */
export const BLOB_STRIDE = 8;
export const RAY_STRIDE = 8;
export const ARC_STRIDE = 8;
export const STROKE_STRIDE = 12;

export interface Scene {
  /** distance, angle, drift, wobble phase, radius, r, g, b */
  readonly blobs: Float32Array;
  /** angle, reach, brightness, white, phase, width px, shimmer rate, 0 */
  readonly rays: Float32Array;
  readonly starRays: Float32Array;
  /** radius, start, length, thickness px, group, alpha, softness, 0 */
  readonly arcs: Float32Array;
  /** per end (angle, base radius, radial offset, along offset) x 2, then width px, group, alpha, shimmer phase */
  readonly strokes: Float32Array;
  /** Phases of the two waves that make the core ring's edge uneven. */
  readonly ringPhases: readonly [number, number];
}

export function buildScene(): Scene {
  const random = seededRandom(SEED);
  const range = (low: number, high: number) => low + (high - low) * random();
  const angle = () => random() * TAU;

  const blobs: number[] = [];
  BLOB_RING_RADII.forEach((distance, ring) => {
    for (let index = 0; index < (BLOBS_PER_RING[ring] ?? 0); index += 1) {
      const pick = random();
      const color = BLOB_COLORS.find(([chance]) => pick < chance) ?? BLOB_COLORS[0];
      blobs.push(
        distance,
        angle(),
        loopRate(range(-BLOB_DRIFT_MAX, BLOB_DRIFT_MAX)),
        angle(),
        range(BLOB_RADIUS_MIN, BLOB_RADIUS_MAX),
        color?.[1] ?? 0,
        color?.[2] ?? 0,
        color?.[3] ?? 0,
      );
    }
  });

  const rays: number[] = [];
  for (let index = 0; index < RAY_COUNT; index += 1) {
    const thick = index % RAY_THICK_EVERY === 0;
    rays.push(
      angle(),
      range(0.6, 1.0),
      thick ? 1 : range(0.3, 0.85),
      range(0, RAY_WHITE_MAX),
      angle(),
      thick ? RAY_THICK_WIDTH_PX : RAY_WIDTH_PX,
      RAY_SHIMMER_RATE,
      0,
    );
  }

  const starRays: number[] = [];
  for (let index = 0; index < STAR_RAY_COUNT; index += 1) {
    starRays.push(angle(), range(0.35, 1.0), 1, STAR_RAY_WHITE, angle(), STAR_RAY_PX, RAY_SHIMMER_RATE, 0);
  }

  const arcs: number[] = [];
  const strokes: number[] = [];
  const pushStroke = (
    a: readonly [number, number, number, number],
    b: readonly [number, number, number, number],
    widthPx: number,
    group: number,
    alpha: number,
    phase: number,
  ) => strokes.push(...a, ...b, widthPx, group, alpha, phase);

  HUD_RINGS.forEach((ring, group) => {
    const shape = ring.shape;
    switch (shape.kind) {
      case 'circle':
        arcs.push(ring.radius, 0, TAU, shape.thicknessPx, group, 1, 0, 0);
        break;
      case 'arcs':
        for (const [start, length] of shape.arcs) {
          arcs.push(ring.radius, start * DEGREES, length * DEGREES, shape.thicknessPx * HUD_GLOW_WIDTH, group, HUD_GLOW_ALPHA, 1, 0);
          arcs.push(ring.radius, start * DEGREES, length * DEGREES, shape.thicknessPx, group, 1, 0, 0);
          if (shape.notches) {
            const from = -HUD_NOTCH_PX / REFERENCE_SIZE;
            const to = (shape.thicknessPx + HUD_NOTCH_PX) / REFERENCE_SIZE;
            pushStroke([start * DEGREES, ring.radius, from, 0], [start * DEGREES, ring.radius, to, 0], 1, group, HUD_NOTCH_ALPHA, 0);
          }
        }
        break;
      case 'dashes': {
        const pitch = TAU / shape.count;
        for (let slot = 0; slot < shape.count; slot += 1) {
          arcs.push(ring.radius, pitch * slot, pitch * shape.duty, shape.thicknessPx, group, 1, 0, 0);
        }
        break;
      }
      case 'ticks':
        for (let slot = 0; slot < shape.count; slot += 1) {
          const theta = (TAU * slot) / shape.count;
          const major = slot % shape.majorEvery === 0;
          const length = major ? shape.majorLen : shape.minorLen;
          pushStroke(
            [theta, ring.radius, 0, 0],
            [theta, ring.radius, length, 0],
            major ? HUD_MAJOR_TICK_PX : HUD_TICK_PX,
            group,
            major ? HUD_MAJOR_TICK_BOOST : 1,
            0,
          );
        }
        break;
    }
  });

  // The darker band behind the glyphs.
  arcs.push(BAND_RADIUS, 0, TAU, BAND_HEIGHT * REFERENCE_SIZE, BACKING_GROUP, 1, 0, 0);

  const halfHeight = BAND_HEIGHT / 2;
  const halfWidth = BAND_WIDTH / 2;
  for (let slot = 0; slot < BAND_GLYPHS; slot += 1) {
    const theta = (TAU * slot) / BAND_GLYPHS;
    const glyph = GLYPHS[(slot * GLYPH_STRIDE) % GLYPHS.length] ?? [];
    const phase = slot * 1.9;
    const at = (x: number, y: number) => [theta, BAND_RADIUS, y * halfHeight, x * halfWidth] as const;
    for (const stroke of glyph) {
      for (const [a, b] of strokeSegments(stroke)) {
        pushStroke(at(a[0], a[1]), at(b[0], b[1]), GLYPH_LINE_PX, GLYPH_GROUP, 1, phase);
      }
      if (stroke.kind === 'dot') {
        // A zero-length stroke draws a round dot; its radius is the glyph line width.
        pushStroke(at(stroke.x, stroke.y), at(stroke.x, stroke.y), GLYPH_LINE_PX * 2, GLYPH_GROUP, 1, phase);
      }
    }
  }

  return {
    blobs: new Float32Array(blobs),
    rays: new Float32Array(rays),
    starRays: new Float32Array(starRays),
    arcs: new Float32Array(arcs),
    strokes: new Float32Array(strokes),
    ringPhases: [angle(), angle()],
  };
}

type Point = readonly [number, number];

/** A stroke's straight segments in its glyph box; arcs are flattened, dots have none. */
function strokeSegments(stroke: Stroke): (readonly [Point, Point])[] {
  switch (stroke.kind) {
    case 'line':
      return [[[stroke.x1, stroke.y1], [stroke.x2, stroke.y2]]];
    case 'dot':
      return [];
    case 'arc': {
      const point = (degrees: number): Point => [
        stroke.cx + stroke.r * Math.cos(degrees * DEGREES),
        stroke.cy + stroke.r * Math.sin(degrees * DEGREES),
      ];
      const segments: (readonly [Point, Point])[] = [];
      for (let step = 0; step < ARC_STEPS; step += 1) {
        const a = stroke.from + ((stroke.to - stroke.from) * step) / ARC_STEPS;
        const b = stroke.from + ((stroke.to - stroke.from) * (step + 1)) / ARC_STEPS;
        segments.push([point(a), point(b)]);
      }
      return segments;
    }
  }
}
