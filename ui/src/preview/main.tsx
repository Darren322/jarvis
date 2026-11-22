/**
 * Dev-only preview of the Orb, the Panels and the timer strip, without Jarvis:
 * run `npx vite` in `ui/`, then open `/preview.html?state=thinking&level=0.08`,
 * `/preview.html?panel=stats&reading=widest&t=2` for the whole screen with the
 * Dashboard, or `/preview.html?panel=stats&timers=five` with the timer strip
 * too. Not part of the production build (Vite builds only `index.html`).
 *
 * URL parameters:
 * - `state`: `offline`, `idle`, `listening`, `transcribing`, `thinking` or `speaking` (default `idle`).
 * - `level`: the raw loudness, clamped to 0 to 1 (default 0.08, typical speech).
 * - `w`, `h`: the Orb's size in CSS pixels (default: the window). Ignored with `panel`.
 * - `x`, `y`: its top-left corner (default 0, 0). Ignored with `panel`.
 * - `panel`: `stats`, `built` (`BUILT_PANEL` in `fixtures.ts`) or `camera` (its
 *   picture fails without Jarvis) shows the whole screen as Jarvis would, with
 *   that Panel and the Orb in its corner.
 * - `timers`: the timer strip's rows, one of `TIMER_FIXTURES` in `fixtures.ts`:
 *   `none`, `one`, `pending`, `running`, `ended`, `five`, `longest`,
 *   `five-pending`. Shows the whole screen, with or without `panel`. Its
 *   countdowns run on the real clock; a tap logs the message it would send.
 * - `reading`: the Dashboard's reading, one of `READINGS` in `fixtures.ts`:
 *   `typical` (default), `no-temperature`, `no-storage`, `waiting`, `service-down`,
 *   `widest`, `widest-unavailable`.
 * - `font`: a CSS font-family for the Panel, to try a wider font than this
 *   machine's system font (the Pi's differs), e.g. `font=Verdana`.
 * - `t`: if set, step the animation to `t` seconds at 60 fps, draw that frame
 *   and stop, so a screenshot is the same every time.
 */

import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import type { Display } from '../link/display';
import type { OrbSource, PanelSource, TimerSource } from '../link/link';
import type { PanelName } from '../link/protocol';
import { clampLevel, VIEWS, type View } from '../orb/look';
import { createOrbRenderer } from '../orb/drawing';
import { type FrameScheduler, Orb } from '../orb/Orb';
import { Screen } from '../panels/Screen';
import { BUILT_PANEL, READINGS, TIMER_FIXTURES } from './fixtures';

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

const PANEL_NAMES: readonly string[] = ['stats', 'built', 'camera'];

function isPanelName(value: string | null): value is PanelName {
  return value !== null && PANEL_NAMES.includes(value);
}

/** A Panel source that shows `panel` (or none) with one fixed reading and never changes. */
function fixedPanel(panel: PanelName | null, readingName: string | null): PanelSource {
  const name = readingName ?? 'typical';
  const stats = Object.hasOwn(READINGS, name) ? (READINGS[name] ?? null) : null;
  return {
    subscribePanel: () => () => undefined,
    shownPanel: panel,
    stats,
    builtPanel: panel === 'built' ? BUILT_PANEL : null,
    closePanel: () => undefined,
  };
}

/** A timer source with the fixture `name` (none if unknown), ending relative to now; a tap logs what Jarvis would get. */
function fixedTimers(name: string | null): TimerSource {
  const nowMs = Date.now();
  const timers = name !== null && Object.hasOwn(TIMER_FIXTURES, name) ? (TIMER_FIXTURES[name]?.(nowMs) ?? []) : [];
  const log = (type: string) => (id: number) => console.info(JSON.stringify({ type, id }));
  return {
    subscribeTimers: () => () => undefined,
    shownTimers: { timers, receivedAtMs: nowMs },
    confirmTimer: log('confirm_timer'),
    rejectTimer: log('reject_timer'),
    cancelTimer: log('cancel_timer'),
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

const font = params.get('font');
if (font !== null) {
  document.documentElement.style.setProperty('--panel-font', font);
}

const panelParam = params.get('panel');
const panel = isPanelName(panelParam) ? panelParam : null;
const timersParam = params.get('timers');

const screen =
  panel !== null || timersParam !== null ? (
    <Screen
      source={{ ...source, ...fixedPanel(panel, params.get('reading')), ...fixedTimers(timersParam) }}
      onTap={() => undefined}
      createRenderer={createOrbRenderer}
      frames={frames}
    />
  ) : (
    <Orb source={source} onTap={() => undefined} size={size} position={position} createRenderer={createOrbRenderer} frames={frames} />
  );
// No StrictMode double effects for a frozen frame: they would step the clock twice.
createRoot(root).render(frames === undefined ? <StrictMode>{screen}</StrictMode> : screen);
