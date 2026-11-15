/**
 * The seam between the Orb's frame loop and whatever draws it.
 *
 * A renderer owns its drawing surfaces: it adds its own canvas (or canvases,
 * for example a 2D-canvas body under a WebGL2 overlay) to the host element it
 * is given, and removes them on `dispose`. Tests pass a fake that records
 * frames, because jsdom has no WebGL.
 */

import type { Look, View } from './look';

/** Everything a renderer needs for one frame. The loop reuses one object, so copy what you keep. */
export interface OrbFrame {
  /** The eased look to draw. */
  readonly look: Readonly<Look>;
  /** The view the Orb is heading for (the look may still be easing towards it). */
  readonly view: View;
  /** The raw level, 0 to 1; 0 unless listening or speaking. */
  readonly level: number;
  /** Seconds of animation since the Orb started, summed from clamped steps. */
  readonly clock: number;
  /** Seconds since the previous drawn frame, clamped to 0 to `MAX_STEP_SECONDS`. */
  readonly dt: number;
  /** Turn of the sparkle cloud and the core ring, radians, 0 to 2π. */
  readonly sparkleAngle: number;
  /** Turn of the glyph band, radians, 0 to 2π. */
  readonly bandAngle: number;
  /** Turn of the HUD rings, radians, 0 to 2π (each ring turns at a multiple of it). */
  readonly hudAngle: number;
  /** Phase of the breath, radians, 0 to 2π. */
  readonly breathPhase: number;
  /** Slow drift of the light rays, radians, 0 to 2π. */
  readonly rayAngle: number;
  /** Drawing area in CSS pixels. */
  readonly width: number;
  readonly height: number;
  /** Device pixels per CSS pixel. */
  readonly pixelRatio: number;
}

export interface OrbRenderer {
  /** Draws one frame. Must not allocate per call. */
  draw(frame: OrbFrame): void;
  /** Frees GPU resources and removes everything it added to the host. */
  dispose(): void;
}

/**
 * Makes a renderer that draws into `host`, or returns null when it cannot (for
 * example, no WebGL2). The Orb then shows nothing but still takes taps.
 */
export type CreateOrbRenderer = (host: HTMLElement) => OrbRenderer | null;
