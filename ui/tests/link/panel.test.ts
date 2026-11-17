import { expect, test } from 'vitest';
import { applyPanelMessage, NO_PANEL, type PanelView } from '../../src/link/panel';
import type { Stats } from '../../src/link/protocol';

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
const HIDE = { type: 'hide_panel' } as const;
const reading = (stats: Stats) => ({ type: 'stats', stats }) as const;

const WAITING: PanelView = { kind: 'stats', reading: null };

test.each([
  // [what happens, before, message, after]
  ['ShowPanel opens the Dashboard with no reading yet', NO_PANEL, SHOW, WAITING],
  ['a reading while the Dashboard shows becomes its reading', WAITING, reading(FIRST), { kind: 'stats', reading: FIRST }],
  ['a newer reading replaces the older one', { kind: 'stats', reading: FIRST }, reading(SECOND), { kind: 'stats', reading: SECOND }],
  ['a reading while no Panel shows is dropped', NO_PANEL, reading(FIRST), NO_PANEL],
  ['HidePanel closes the Dashboard and drops its reading', { kind: 'stats', reading: FIRST }, HIDE, NO_PANEL],
  ['HidePanel with no Panel shown changes nothing', NO_PANEL, HIDE, NO_PANEL],
] as const)('%s', (_, before, message, after) => {
  expect(applyPanelMessage(before, message)).toEqual(after);
});

test('showing the Dashboard while it shows keeps its reading and returns the same view', () => {
  const view: PanelView = { kind: 'stats', reading: FIRST };
  expect(applyPanelMessage(view, SHOW)).toBe(view);
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
