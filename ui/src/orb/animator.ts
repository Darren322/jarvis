/**
 * The Orb's frame loop body, with no React and no browser timing: it is called
 * with a timestamp, reads the link's store, eases the look and asks the
 * renderer to draw. Ported from `Orb::draw` and `Orb::advance` in
 * `crates/jarvis-orb/src/orb.rs`.
 */

import type { OrbSource } from '../link/link';
import { easeFactor, emptyLook, type Look, lerpLookInto, lookInto, type View } from './look';
import type { OrbFrame, OrbRenderer } from './renderer';

/** The longest step the animation takes in one frame, in seconds, so a stall never makes the Orb jump. */
export const MAX_STEP_SECONDS = 0.1;

/** While idle or offline the Orb draws at most 30 times a second (jarvis-ui D5). */
export const IDLE_FRAME_INTERVAL = 1 / 30;

/**
 * How early a throttled frame may come and still be drawn, in seconds. Display
 * frames jitter by a millisecond or two, so without this a 60 Hz display would
 * sometimes skip two frames in a row while idle.
 */
const FRAME_SLACK = 0.004;

/** How fast the light rays drift, radians per second. */
const RAY_TURN_RATE = 0.012;

const TAU = Math.PI * 2;

/** An angle wrapped into 0 to 2π, so it keeps its precision however long the Orb runs. */
function wrapAngle(angle: number): number {
  return angle - TAU * Math.floor(angle / TAU);
}

/** True for the views that may draw at the idle frame rate. */
function isQuiet(view: View): boolean {
  return view === 'idle' || view === 'offline';
}

type MutableFrame = { -readonly [K in keyof OrbFrame]: OrbFrame[K] };

export class OrbAnimator {
  readonly #source: OrbSource;
  readonly #renderer: OrbRenderer;
  readonly #current: Look = emptyLook();
  readonly #target: Look = emptyLook();
  readonly #frame: MutableFrame;
  #lastDrawn: number | null = null;

  constructor(source: OrbSource, renderer: OrbRenderer) {
    this.#source = source;
    this.#renderer = renderer;
    this.#frame = {
      look: this.#current,
      view: 'offline',
      level: 0,
      clock: 0,
      dt: 0,
      sparkleAngle: 0,
      bandAngle: 0,
      hudAngle: 0,
      breathPhase: 0,
      rayAngle: 0,
      width: 0,
      height: 0,
      pixelRatio: 1,
    };
  }

  /**
   * Called once per display frame with the time in seconds. Draws unless the
   * Orb is idle or offline and less than `IDLE_FRAME_INTERVAL` has passed since
   * the last drawn frame. Returns whether it drew.
   */
  frame(now: number, width: number, height: number, pixelRatio: number): boolean {
    const display = this.#source.display;
    const view: View = display.kind === 'offline' ? 'offline' : display.state;
    const level = display.kind === 'offline' ? 0 : display.level;
    lookInto(this.#target, view, level);

    let dt = 0;
    if (this.#lastDrawn === null) {
      // The first frame shows the target at once instead of easing in from nothing.
      lerpLookInto(this.#current, this.#target, this.#target, 1);
    } else {
      const elapsed = now - this.#lastDrawn;
      if (isQuiet(view) && elapsed + FRAME_SLACK < IDLE_FRAME_INTERVAL) {
        return false;
      }
      dt = Number.isNaN(elapsed) ? 0 : Math.min(MAX_STEP_SECONDS, Math.max(0, elapsed));
      lerpLookInto(this.#current, this.#current, this.#target, easeFactor(dt));
    }
    this.#lastDrawn = now;
    this.#advance(dt);

    const frame = this.#frame;
    frame.view = view;
    frame.level = level;
    frame.dt = dt;
    frame.width = width;
    frame.height = height;
    frame.pixelRatio = pixelRatio;
    this.#renderer.draw(frame);
    return true;
  }

  #advance(dt: number): void {
    const look = this.#current;
    const frame = this.#frame;
    frame.clock += dt;
    frame.sparkleAngle = wrapAngle(frame.sparkleAngle + look.sparkleSpin * dt);
    frame.bandAngle = wrapAngle(frame.bandAngle + look.bandSpin * dt);
    frame.hudAngle = wrapAngle(frame.hudAngle + look.hudSpin * dt);
    frame.breathPhase = wrapAngle(frame.breathPhase + TAU * look.breathRate * dt);
    frame.rayAngle = wrapAngle(frame.rayAngle + RAY_TURN_RATE * dt);
  }
}
