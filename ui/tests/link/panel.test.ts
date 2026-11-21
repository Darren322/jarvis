import { expect, test } from 'vitest';
import { applyPanelMessage, NO_PANEL, type PanelView } from '../../src/link/panel';
import type { BuiltPanel, Stats } from '../../src/link/protocol';

const FIRST: Stats = {
  cpuPercent: 12.5,
  temperatureC: 55,
  ramUsedBytes: 2,
  ramTotalBytes: 8,
  storage: null,
  llama: 'up',
  bge: 'down',
};
const SECOND: Stats = { ...FIRST, cpuPercent: 40 };

const SHOW = { type: 'show_panel', panel: 'stats' } as const;
const SHOW_CAMERA = { type: 'show_panel', panel: 'camera' } as const;
const HIDE = { type: 'hide_panel' } as const;
const reading = (stats: Stats) => ({ type: 'stats', stats }) as const;

const WAITING: PanelView = { kind: 'stats', reading: null };
const CAMERA: PanelView = { kind: 'camera' };

test.each([
  // [what happens, before, message, after]
  ['ShowPanel opens the Dashboard with no reading yet', NO_PANEL, SHOW, WAITING],
  ['a reading while the Dashboard shows becomes its reading', WAITING, reading(FIRST), { kind: 'stats', reading: FIRST }],
  ['a newer reading replaces the older one', { kind: 'stats', reading: FIRST }, reading(SECOND), { kind: 'stats', reading: SECOND }],
  ['a reading while no Panel shows is dropped', NO_PANEL, reading(FIRST), NO_PANEL],
  ['HidePanel closes the Dashboard and drops its reading', { kind: 'stats', reading: FIRST }, HIDE, NO_PANEL],
  ['HidePanel with no Panel shown changes nothing', NO_PANEL, HIDE, NO_PANEL],
  ['ShowPanel camera opens the camera', NO_PANEL, SHOW_CAMERA, CAMERA],
  ['the camera replaces the Dashboard and drops its reading', { kind: 'stats', reading: FIRST }, SHOW_CAMERA, CAMERA],
  ['the Dashboard replaces the camera and waits for a reading', CAMERA, SHOW, WAITING],
  ['a reading while the camera shows is dropped', CAMERA, reading(FIRST), CAMERA],
  ['HidePanel closes the camera', CAMERA, HIDE, NO_PANEL],
] as const)('%s', (_, before, message, after) => {
  expect(applyPanelMessage(before, message)).toEqual(after);
});

test('showing the Dashboard while it shows keeps its reading and returns the same view', () => {
  const view: PanelView = { kind: 'stats', reading: FIRST };
  expect(applyPanelMessage(view, SHOW)).toBe(view);
});

test('showing the camera while it shows returns the same view, so nothing re-renders', () => {
  const view: PanelView = { kind: 'camera' };
  expect(applyPanelMessage(view, SHOW_CAMERA)).toBe(view);
  expect(applyPanelMessage(view, reading(FIRST))).toBe(view);
});

test('reopening after HidePanel never shows the old reading', () => {
  let view: PanelView = NO_PANEL;
  for (const message of [SHOW, reading(FIRST), HIDE, SHOW]) {
    view = applyPanelMessage(view, message);
  }
  expect(view).toEqual(WAITING);
});

test('a dropped reading or a repeated HidePanel returns the same view, so nothing re-renders', () => {
  expect(applyPanelMessage(NO_PANEL, reading(FIRST))).toBe(NO_PANEL);
  expect(applyPanelMessage(NO_PANEL, HIDE)).toBe(NO_PANEL);
});

const FACTS: BuiltPanel = { title: 'Facts', parts: [{ kind: 'list', title: 'Facts', items: ['Darren likes tea'] }] };
const GAUGES: BuiltPanel = { title: 'Pi', parts: [{ kind: 'gauge', source: 'cpu' }] };
const build = (panel: BuiltPanel) => ({ type: 'show_built_panel', panel }) as const;
const BUILT_WAITING: PanelView = { kind: 'built', panel: GAUGES, reading: null };

test.each([
  ['ShowBuiltPanel opens its parts with no reading yet', NO_PANEL, build(GAUGES), BUILT_WAITING],
  ['a reading while a built Panel shows feeds its gauges', BUILT_WAITING, reading(FIRST), { ...BUILT_WAITING, reading: FIRST }],
  ['a built Panel replaces the Dashboard and drops its reading', { kind: 'stats', reading: FIRST }, build(GAUGES), BUILT_WAITING],
  ['a built Panel replaces the camera', CAMERA, build(GAUGES), BUILT_WAITING],
  ['a new built Panel replaces the old one and its reading', { ...BUILT_WAITING, reading: FIRST }, build(FACTS), { kind: 'built', panel: FACTS, reading: null }],
  ['the Dashboard replaces a built Panel', { ...BUILT_WAITING, reading: FIRST }, SHOW, WAITING],
  ['the camera replaces a built Panel', BUILT_WAITING, SHOW_CAMERA, CAMERA],
  ['HidePanel closes a built Panel', { ...BUILT_WAITING, reading: FIRST }, HIDE, NO_PANEL],
] as const)('%s', (_, before, message, after) => {
  expect(applyPanelMessage(before, message)).toEqual(after);
});
