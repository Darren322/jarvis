/**
 * Dev-only preview of the Orb, without Jarvis: run `npx vite` in `ui/`, then open
 * `/preview.html?state=thinking&level=0.08`. Not part of the production build
 * (Vite builds only `index.html`).
 *
 * URL parameters:
 * - `state`: `offline`, `idle`, `listening`, `transcribing`, `thinking` or `speaking` (default `idle`).
 * - `level`: the raw loudness, clamped to 0 to 1 (default 0.08, typical speech).
 * - `w`, `h`: the Orb's size in CSS pixels (default: the window).
 * - `x`, `y`: its top-left corner (default 0, 0).
 * - `t`: if set, step the animation to `t` seconds at 60 fps, draw that frame
 *   and stop, so a screenshot is the same every time.
 */

import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import type { Display } from '../link/display';
import type { OrbSource } from '../link/link';
import { clampLevel, VIEWS, type View } from '../orb/look';
import { createOrbRenderer } from '../orb/drawing';
import { type FrameScheduler, Orb } from '../orb/Orb';

/** The raw level used when the URL gives none: a typical speech loudness. */
const DEFAULT_LEVEL = 0.08;
/** Frame step of a frozen preview, in milliseconds (60 fps). */
const FROZEN_STEP_MS = 1000 / 60;

function isView(value: string | null): value is View {
  return value !== null && (VIEWS as readonly string[]).includes(value);
}

function numberParam(params: URLSearchParams, name: string): number | null {
  const raw = params.get(name);
  if (raw === null || raw.trim() === '') {
    return null;
  }
  const value = Number(raw);
  return Number.isFinite(value) ? value : null;
}

function displayFor(view: View, level: number): Display {
  return view === 'offline' ? { kind: 'offline' } : { kind: 'online', state: view, level };
}

/** Steps through fixed frame times up to `seconds`, then stops asking for frames. */
function frozenFrames(seconds: number): FrameScheduler {
  const last = Math.max(0, Math.round((seconds * 1000) / FROZEN_STEP_MS));
  let step = 0;
  let cancelled = false;
  return {
    request(callback) {
      if (step <= last) {
        const time = step * FROZEN_STEP_MS;
        step += 1;
        queueMicrotask(() => {
          if (!cancelled) {
            callback(time);
          }
        });
      }
      return step;
    },
    cancel() {
      cancelled = true;
    },
  };
}

const params = new URLSearchParams(window.location.search);
const stateParam = params.get('state');
const view: View = isView(stateParam) ? stateParam : 'idle';
const level = clampLevel(numberParam(params, 'level') ?? DEFAULT_LEVEL);
const source: OrbSource = { display: displayFor(view, level) };
const size = {
  width: numberParam(params, 'w') ?? window.innerWidth,
  height: numberParam(params, 'h') ?? window.innerHeight,
};
const position = { left: numberParam(params, 'x') ?? 0, top: numberParam(params, 'y') ?? 0 };
const freezeAt = numberParam(params, 't');
const frames = freezeAt === null ? undefined : frozenFrames(freezeAt);

const root = document.getElementById('root');
if (!root) {
  throw new Error('preview.html has no #root element');
}

// No StrictMode double effects for a frozen frame: they would step the clock twice.
const orb = (
  <Orb source={source} onTap={() => undefined} size={size} position={position} createRenderer={createOrbRenderer} frames={frames} />
);
createRoot(root).render(frames === undefined ? <StrictMode>{orb}</StrictMode> : orb);
