/**
 * The renderer the app uses: two canvases in the Orb's host element.
 *
 * - Behind: one WebGL2 canvas over the whole area with the D1 touches from the
 *   approved macroquad orb (blobs, light rays, HUD rings, glyph band, warm core).
 * - In front: a transparent 2D canvas, a square around the centre, with the
 *   body: the thinking-orbs dot sphere (MIT, Jakub Antalik,
 *   https://libraries.dev/orbs), whose motion changes with the state.
 *
 * The body sits in front so its near dots read over the core's glow, like a
 * sphere of light in front of a warm centre.
 */

import { Body } from './body/body';
import { Backdrop } from './gl/backdrop';
import type { CreateOrbRenderer, OrbFrame, OrbRenderer } from './renderer';

/** The body canvas's side as a fraction of the shorter side. The sphere fills about 0.82 of it. */
export const BODY_FRACTION = 0.66;

function layerCanvas(): HTMLCanvasElement {
  const canvas = document.createElement('canvas');
  canvas.style.position = 'absolute';
  canvas.style.display = 'block';
  canvas.style.pointerEvents = 'none';
  return canvas;
}

class LayeredRenderer implements OrbRenderer {
  readonly #root: HTMLDivElement;
  readonly #glCanvas: HTMLCanvasElement;
  readonly #backdrop: Backdrop | null;
  readonly #body: Body | null;
  #bodySide = -1;
  #bodyLeft = -1;
  #bodyTop = -1;

  constructor(root: HTMLDivElement, glCanvas: HTMLCanvasElement, backdrop: Backdrop | null, body: Body | null) {
    this.#root = root;
    this.#glCanvas = glCanvas;
    this.#backdrop = backdrop;
    this.#body = body;
  }

  draw(frame: OrbFrame): void {
    const width = Math.max(1, Math.round(frame.width * frame.pixelRatio));
    const height = Math.max(1, Math.round(frame.height * frame.pixelRatio));
    if (this.#backdrop !== null) {
      const canvas = this.#glCanvas;
      if (canvas.width !== width || canvas.height !== height) {
        canvas.width = width;
        canvas.height = height;
      }
      this.#backdrop.draw(frame, width, height);
    }
    if (this.#body !== null) {
      const side = Math.round(Math.min(frame.width, frame.height) * BODY_FRACTION);
      const left = Math.round((frame.width - side) / 2);
      const top = Math.round((frame.height - side) / 2);
      if (side !== this.#bodySide || left !== this.#bodyLeft || top !== this.#bodyTop) {
        const style = this.#body.canvas.style;
        style.width = `${side}px`;
        style.height = `${side}px`;
        style.left = `${left}px`;
        style.top = `${top}px`;
        this.#bodySide = side;
        this.#bodyLeft = left;
        this.#bodyTop = top;
      }
      this.#body.draw(frame, side);
    }
  }

  dispose(): void {
    this.#backdrop?.dispose();
    this.#root.remove();
  }
}

export const createOrbRenderer: CreateOrbRenderer = (host) => {
  const root = document.createElement('div');
  root.style.position = 'absolute';
  root.style.inset = '0';
  root.style.background = '#000';
  root.style.pointerEvents = 'none';

  const glCanvas = layerCanvas();
  glCanvas.style.inset = '0';
  glCanvas.style.width = '100%';
  glCanvas.style.height = '100%';
  const gl = glCanvas.getContext('webgl2', { alpha: false, antialias: false, depth: false, stencil: false });
  const backdrop = gl === null ? null : Backdrop.create(gl);

  const bodyCanvas = layerCanvas();
  const ctx = bodyCanvas.getContext('2d');
  const body = ctx === null ? null : new Body(bodyCanvas, ctx);

  if (backdrop === null && body === null) {
    return null;
  }
  if (backdrop !== null) {
    root.append(glCanvas);
  }
  if (body !== null) {
    root.append(bodyCanvas);
  }
  host.append(root);
  return new LayeredRenderer(root, glCanvas, backdrop, body);
};
