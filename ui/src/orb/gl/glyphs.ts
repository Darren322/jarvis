/**
 * The invented alphabet of the glyph band: ten tall marks made of lines, arcs
 * and dots. They stand for nothing and are not drawn from any real script.
 * Ported from `GLYPHS` in `crates/jarvis-orb/src/orb.rs`.
 *
 * Each stroke lives in a box running -1 to 1 across (x, along the ring) and
 * from the band's centre side to its outer side (y). Angles are in degrees.
 */

export type Stroke =
  | { readonly kind: 'line'; readonly x1: number; readonly y1: number; readonly x2: number; readonly y2: number }
  | { readonly kind: 'arc'; readonly cx: number; readonly cy: number; readonly r: number; readonly from: number; readonly to: number }
  | { readonly kind: 'dot'; readonly x: number; readonly y: number };

const line = (x1: number, y1: number, x2: number, y2: number): Stroke => ({ kind: 'line', x1, y1, x2, y2 });
const arc = (cx: number, cy: number, r: number, from: number, to: number): Stroke => ({ kind: 'arc', cx, cy, r, from, to });
const dot = (x: number, y: number): Stroke => ({ kind: 'dot', x, y });

export const GLYPHS: readonly (readonly Stroke[])[] = [
  // A stem with a pointed crown, a bar and a dot below.
  [line(0, -1, 0, 1), line(-0.8, 0.55, 0, 0.95), line(0, 0.95, 0.8, 0.55), line(-0.8, -0.3, 0.8, -0.3), dot(0, -0.7)],
  // Two uneven stems joined by a slanting bar.
  [line(-0.6, -1, -0.6, 1), line(0.6, -1, 0.6, 0.6), line(-0.6, 0.2, 0.6, -0.3), dot(0.6, 0.92)],
  // A diamond on a stem with a foot bar.
  [line(0, 1, 0.7, 0.5), line(0.7, 0.5, 0, 0), line(0, 0, -0.7, 0.5), line(-0.7, 0.5, 0, 1), line(0, 0, 0, -1), line(-0.5, -0.6, 0.5, -0.6)],
  // An arch on two legs with a seed beneath.
  [arc(0, 0.35, 0.75, 180, 360), line(-0.75, 0.35, -0.75, -0.8), line(0.75, 0.35, 0.75, -1), dot(0, -0.2)],
  // A zigzag.
  [line(-0.7, 1, 0.7, 0.5), line(0.7, 0.5, -0.7, 0), line(-0.7, 0, 0.7, -0.5), line(0.7, -0.5, -0.7, -1)],
  // A tine with two unequal branches and a dot above.
  [line(0, -1, 0, 0.7), line(0, 0.2, -0.8, 0.8), line(0, -0.2, 0.8, 0.4), dot(0, 0.95)],
  // A ladder of two rails with dots at each end.
  [line(-0.5, -1, -0.5, 1), line(0.5, -1, 0.5, 1), line(-0.5, 0.5, 0.5, 0.5), line(-0.5, -0.2, 0.5, -0.2), dot(0, 0.85), dot(0, -0.7)],
  // A staff ending in a hook, with a serif and a dot.
  [line(0.3, -1, 0.3, 0.4), arc(-0.15, 0.45, 0.45, 0, 180), line(-0.6, -0.5, 0.3, -0.5), dot(-0.6, -0.8)],
  // A trident.
  [line(0, -1, 0, 0.6), line(-0.8, 0.9, -0.8, 0.1), line(-0.8, 0.1, 0, -0.1), line(0.8, 0.9, 0.8, 0.1), line(0.8, 0.1, 0, -0.1), dot(0, 0.9)],
  // A cross-brace on a stem with a foot bar.
  [line(-0.7, 0.9, 0.7, -0.2), line(0.7, 0.9, -0.7, -0.2), line(0, -0.2, 0, -1), line(-0.6, -0.7, 0.6, -0.7), dot(0, 0.95)],
];

/** Segments an arc is flattened into. */
export const ARC_STEPS = 6;
