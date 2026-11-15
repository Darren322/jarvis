/**
 * The Orb's body: the thinking-orbs dot sphere (MIT, Jakub Antalik,
 * https://libraries.dev/orbs) on a 2D canvas, tinted with the look's palette.
 *
 * The engine (`MODE_FRAMES[mode](size, t, opts)`) gives each frame's dots and
 * lines; this file paints them. Its own painter sets a colour string and fills
 * one path per dot. This one sorts dots into a few ink and alpha buckets and
 * fills each bucket as one path, so a frame costs about two dozen fills
 * instead of one per dot. The engine allocates its frame objects every call;
 * this painter allocates nothing per frame except colour strings while the
 * palette is easing.
 */

import { type Dot, type Line, MODE_FRAMES, type ModeOpts, type OrbFrame as EngineFrame } from 'thinking-orbs/engine';
import { type Rgb, VIEWS, type View } from '../look';
import type { OrbFrame } from '../renderer';
import { bodyMode, bodySpeed, countScale, writeBodyOpts } from './modes';

/** Brightness steps a dot's colour is rounded to. */
const INK_LEVELS = 6;
/** Opacity steps a dot's alpha is rounded to. */
const ALPHA_LEVELS = 4;
const BUCKETS = INK_LEVELS * ALPHA_LEVELS;
/** The most dots or lines painted per layer; the engine's modes here draw under 400. */
const MAX_MARKS = 2048;
/** How long the body crossfades from one mode to the next, in seconds. */
const FADE_SECONDS = 0.45;
/**
 * How far the near (bright) dots' colour moves from the HUD colour towards
 * white. Icy, not warm: cool dots stand out against the warm core behind them.
 */
const NEAR_WHITE = 0.7;
/** How far the links' colour moves from the HUD colour towards white. */
const LINK_WHITE = 0.35;
/** The engine's link opacities were tuned for dark ink on paper; on black they need lifting. */
const LINK_ALPHA_GAIN = 1.8;
/** Line width is this many times the engine's own. */
const LINK_WIDTH_SCALE = 1.3;
/** The dimmest ink is this share of the brightest, so far dots still read. */
const INK_FLOOR = 0.45;
/** Body opacity is `BODY_ALPHA_BASE + BODY_ALPHA_GAIN * lum`, clamped to 1. */
const BODY_ALPHA_BASE = 0.25;
const BODY_ALPHA_GAIN = 0.85;
/** Brightness swing of one breath, as in the WebGL layer. */
const BREATH_BRIGHTNESS = 0.15;
/** The whole Orb swells by this fraction of its radius at a full pulse. */
const PULSE_SWELL = 0.04;
const TAU = Math.PI * 2;

function clampUnit(value: number): number {
  return Math.min(1, Math.max(0, value));
}

/** One channel of an ink: `t` of the way from the far colour to its whitened self, times `lum`. */
function inkChannel(far: number, t: number, lum: number): number {
  const near = far + (1 - far) * NEAR_WHITE;
  return (far + (near - far) * t) * lum;
}

/** A cached `rgb()` string, rebuilt only when its rounded channels change. */
class ColorSlot {
  #key = -1;
  value = 'rgb(0,0,0)';

  set(r: number, g: number, b: number): string {
    const ri = Math.round(clampUnit(r) * 255);
    const gi = Math.round(clampUnit(g) * 255);
    const bi = Math.round(clampUnit(b) * 255);
    const key = (ri << 16) | (gi << 8) | bi;
    if (key !== this.#key) {
      this.#key = key;
      this.value = `rgb(${ri},${gi},${bi})`;
    }
    return this.value;
  }
}

export class Body {
  readonly canvas: HTMLCanvasElement;
  readonly #ctx: CanvasRenderingContext2D;
  readonly #opts: Record<View, ModeOpts>;
  readonly #inks = Array.from({ length: INK_LEVELS }, () => new ColorSlot());
  readonly #linkInk = new ColorSlot();
  readonly #counts = new Int32Array(BUCKETS);
  readonly #items = new Int32Array(BUCKETS * MAX_MARKS);
  #time = 0;
  #view: View | null = null;
  #previous: View | null = null;
  #fade = 1;

  constructor(canvas: HTMLCanvasElement, ctx: CanvasRenderingContext2D) {
    this.canvas = canvas;
    this.#ctx = ctx;
    this.#opts = Object.fromEntries(VIEWS.map((view) => [view, {}])) as Record<View, ModeOpts>;
  }

  /** Draws one frame on a square body canvas of `side` CSS pixels. */
  draw(frame: OrbFrame, side: number): void {
    const { canvas } = this;
    const ctx = this.#ctx;
    const pixels = Math.max(1, Math.round(side * frame.pixelRatio));
    if (canvas.width !== pixels || canvas.height !== pixels) {
      canvas.width = pixels;
      canvas.height = pixels;
    }
    this.#advance(frame);

    const look = frame.look;
    const breath = Math.sin(frame.breathPhase);
    const lum = look.brightness * (1 + BREATH_BRIGHTNESS * breath);
    const swell = 1 + look.breathDepth * breath + look.nodePulse * PULSE_SWELL;
    const alpha = clampUnit(BODY_ALPHA_BASE + BODY_ALPHA_GAIN * lum);
    this.#setInks(look.hudColor);

    ctx.setTransform(1, 0, 0, 1, 0, 0);
    ctx.clearRect(0, 0, pixels, pixels);
    const scale = (pixels / side) * swell;
    ctx.setTransform(scale, 0, 0, scale, (pixels * (1 - swell)) / 2, (pixels * (1 - swell)) / 2);

    const scaleCount = countScale(side);
    const current = this.#view ?? frame.view;
    if (this.#previous !== null && this.#fade < 1) {
      this.#paintView(this.#previous, frame.level, side, scaleCount, alpha * (1 - this.#fade));
    }
    this.#paintView(current, frame.level, side, scaleCount, alpha * Math.min(1, this.#fade));
  }

  #advance(frame: OrbFrame): void {
    if (this.#view !== frame.view) {
      // A first frame shows its view at once; a change crossfades.
      this.#previous = this.#view;
      this.#fade = this.#view === null ? 1 : 0;
      this.#view = frame.view;
    }
    this.#fade = Math.min(1, this.#fade + frame.dt / FADE_SECONDS);
    this.#time += frame.dt * bodySpeed(frame.view, frame.level);
  }

  #setInks(far: Rgb): void {
    for (let level = 0; level < INK_LEVELS; level += 1) {
      const t = (level + 0.5) / INK_LEVELS;
      const lum = INK_FLOOR + (1 - INK_FLOOR) * t;
      this.#inks[level]?.set(inkChannel(far[0], t, lum), inkChannel(far[1], t, lum), inkChannel(far[2], t, lum));
    }
    this.#linkInk.set(
      far[0] + (1 - far[0]) * LINK_WHITE,
      far[1] + (1 - far[1]) * LINK_WHITE,
      far[2] + (1 - far[2]) * LINK_WHITE,
    );
  }

  #paintView(view: View, level: number, side: number, scale: number, alpha: number): void {
    if (alpha < 0.01) {
      return;
    }
    const opts = writeBodyOpts(this.#opts[view], view, level, scale);
    const geometry: EngineFrame = MODE_FRAMES[bodyMode(view)](side, this.#time, opts);
    this.#paintLines(geometry.lines, alpha);
    this.#paintDots(geometry.dots, alpha);
  }

  #paintLines(lines: readonly Line[], alpha: number): void {
    const total = Math.min(lines.length, MAX_MARKS);
    if (total === 0) {
      return;
    }
    const ctx = this.#ctx;
    const counts = this.#counts;
    const items = this.#items;
    counts.fill(0, 0, ALPHA_LEVELS);
    let width = 0;
    for (let index = 0; index < total; index += 1) {
      const line = lines[index];
      if (line === undefined) {
        continue;
      }
      width = Math.max(width, line.w);
      const step = Math.min(ALPHA_LEVELS - 1, Math.floor(clampUnit((line.a ?? 1) * LINK_ALPHA_GAIN) * ALPHA_LEVELS));
      items[step * MAX_MARKS + (counts[step] ?? 0)] = index;
      counts[step] = (counts[step] ?? 0) + 1;
    }
    ctx.strokeStyle = this.#linkInk.value;
    ctx.lineWidth = width * LINK_WIDTH_SCALE;
    for (let step = 0; step < ALPHA_LEVELS; step += 1) {
      const count = counts[step] ?? 0;
      if (count === 0) {
        continue;
      }
      ctx.globalAlpha = alpha * ((step + 1) / ALPHA_LEVELS);
      ctx.beginPath();
      for (let slot = 0; slot < count; slot += 1) {
        const line = lines[items[step * MAX_MARKS + slot] ?? 0];
        if (line !== undefined) {
          ctx.moveTo(line.x1, line.y1);
          ctx.lineTo(line.x2, line.y2);
        }
      }
      ctx.stroke();
    }
    ctx.globalAlpha = 1;
  }

  #paintDots(dots: readonly Dot[], alpha: number): void {
    const total = Math.min(dots.length, MAX_MARKS);
    const ctx = this.#ctx;
    const counts = this.#counts;
    const items = this.#items;
    counts.fill(0);
    for (let index = 0; index < total; index += 1) {
      const dot = dots[index];
      if (dot === undefined) {
        continue;
      }
      // The engine's ink: 0 is the darkest ink on paper, so on our dark screen 1 - white is the
      // brightness; the square root lifts the mid tones, which read too dark on black.
      const ink = Math.min(INK_LEVELS - 1, Math.floor(Math.sqrt(clampUnit(1 - dot.white)) * INK_LEVELS));
      const step = Math.min(ALPHA_LEVELS - 1, Math.floor(clampUnit(dot.a ?? 1) * ALPHA_LEVELS));
      const bucket = ink * ALPHA_LEVELS + step;
      items[bucket * MAX_MARKS + (counts[bucket] ?? 0)] = index;
      counts[bucket] = (counts[bucket] ?? 0) + 1;
    }
    // Dim far ink first, bright near ink last, so the near side reads on top.
    for (let bucket = 0; bucket < BUCKETS; bucket += 1) {
      const count = counts[bucket] ?? 0;
      if (count === 0) {
        continue;
      }
      const ink = Math.floor(bucket / ALPHA_LEVELS);
      const step = bucket % ALPHA_LEVELS;
      ctx.fillStyle = this.#inks[ink]?.value ?? 'rgb(0,0,0)';
      ctx.globalAlpha = alpha * ((step + 1) / ALPHA_LEVELS);
      ctx.beginPath();
      for (let slot = 0; slot < count; slot += 1) {
        const dot = dots[items[bucket * MAX_MARKS + slot] ?? 0];
        if (dot !== undefined) {
          ctx.moveTo(dot.x + dot.r, dot.y);
          ctx.arc(dot.x, dot.y, dot.r, 0, TAU);
        }
      }
      ctx.fill();
    }
    ctx.globalAlpha = 1;
  }
}
