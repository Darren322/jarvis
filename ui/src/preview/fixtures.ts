/**
 * Fixed Stats readings, a built Panel and timers for the dev-only preview, so
 * the Panels and the timer strip can be seen and screenshotted without Jarvis.
 */

import type { BuiltPanel, Stats, StorageSpace, Timer } from '../link/protocol';

/** About what `free -h` calls 7.9 GiB: the 8 GB Pi's total RAM. */
const PI_RAM_TOTAL_BYTES = 8_464_105_472;

/** The 8 GB Pi's 256 GB drive as `sysinfo` read it on 2026-11-16: "44 / 251 GB". */
const PI_STORAGE: StorageSpace = { availableBytes: 207_517_036_544, totalBytes: 251_416_899_584 };

/** A full drive of 9,999 GB: the widest storage text, "9999 / 9999 GB". */
const WIDEST_STORAGE: StorageSpace = { availableBytes: 0, totalBytes: 9_999_000_000_000 };

const TYPICAL: Stats = {
  cpuPercent: 12.5,
  temperatureC: 55.3,
  ramUsedBytes: 2_469_606_195,
  ramTotalBytes: PI_RAM_TOTAL_BYTES,
  storage: PI_STORAGE,
  llama: 'up',
  bge: 'up',
};

/** The widest text each value can take: CPU "100%", temperature "100°C", memory "7.9 / 7.9 GB", storage "9999 / 9999 GB". */
const WIDEST: Stats = {
  cpuPercent: 100,
  temperatureC: 99.6,
  ramUsedBytes: PI_RAM_TOTAL_BYTES,
  ramTotalBytes: PI_RAM_TOTAL_BYTES,
  storage: WIDEST_STORAGE,
  llama: 'down',
  bge: 'down',
};

/** Each fixture by its `reading` URL parameter; `waiting` is the Dashboard before its first reading. */
export const READINGS: Readonly<Record<string, Stats | null>> = {
  typical: TYPICAL,
  'no-temperature': { ...TYPICAL, temperatureC: null },
  'no-storage': { ...TYPICAL, storage: null },
  waiting: null,
  'service-down': { ...TYPICAL, bge: 'down' },
  widest: WIDEST,
  // The widest values with both temperature and storage "Unavailable".
  'widest-unavailable': { ...WIDEST, temperatureC: null, storage: null },
};

/** A built Panel with every kind of part, to see a built Panel beside the timer strip. */
export const BUILT_PANEL: BuiltPanel = {
  title: 'Shopping',
  parts: [
    { kind: 'list', title: 'To buy', items: ['Milk', 'Eggs', 'Bread', 'Coffee beans'] },
    { kind: 'card', title: 'Note', text: 'The shop on the corner closes at 9 pm on Sundays.' },
    { kind: 'gauge', source: 'cpu' },
  ],
};

const MINUTE_MS = 60_000;
const HOUR_MS = 60 * MINUTE_MS;

/** A label of exactly `MAX_TIMER_LABEL_CHARS` (60) characters: the longest Jarvis sends. */
export const LONGEST_TIMER_LABEL = 'Call Mum back about Sunday lunch and ask what to bring along';

/**
 * Each set of timers by its `timers` URL parameter, as a function of the
 * preview's clock (Unix milliseconds) so the countdowns are live.
 */
export const TIMER_FIXTURES: Readonly<Record<string, (nowMs: number) => readonly Timer[]>> = {
  none: () => [],
  one: (nowMs) => [{ id: 1, label: 'Time to sleep', endsAtMs: nowMs + 25 * MINUTE_MS, state: 'running' }],
  pending: (nowMs) => [{ id: 1, label: 'Time to sleep', endsAtMs: nowMs + 2 * HOUR_MS, state: 'pending' }],
  running: (nowMs) => [{ id: 1, label: 'Time to do work', endsAtMs: nowMs + HOUR_MS - 1000, state: 'running' }],
  ended: (nowMs) => [{ id: 1, label: 'Time to sleep', endsAtMs: nowMs - 20_000, state: 'ended' }],
  // The most timers (MAX_TIMERS), every state, an unlabelled one and one about to end.
  five: (nowMs) => [
    { id: 1, label: 'Time to sleep', endsAtMs: nowMs + 2 * HOUR_MS + 5 * MINUTE_MS, state: 'pending' },
    { id: 2, label: 'Time to do work', endsAtMs: nowMs + 25 * MINUTE_MS, state: 'running' },
    { id: 3, label: null, endsAtMs: nowMs + 75 * MINUTE_MS, state: 'running' },
    { id: 4, label: 'Tea', endsAtMs: nowMs + 3000, state: 'running' },
    { id: 5, label: 'Pasta', endsAtMs: nowMs - 40_000, state: 'ended' },
  ],
  // The longest label in each state, and 23:59:59, the widest countdown.
  longest: (nowMs) => [
    { id: 1, label: LONGEST_TIMER_LABEL, endsAtMs: nowMs + 24 * HOUR_MS - 1000, state: 'running' },
    { id: 2, label: LONGEST_TIMER_LABEL, endsAtMs: nowMs + HOUR_MS, state: 'pending' },
    { id: 3, label: LONGEST_TIMER_LABEL, endsAtMs: nowMs, state: 'ended' },
  ],
  // Five pending rows with the longest label: the tallest the strip can be.
  'five-pending': (nowMs) =>
    [1, 2, 3, 4, 5].map((id) => ({
      id,
      label: LONGEST_TIMER_LABEL,
      endsAtMs: nowMs + id * HOUR_MS,
      state: 'pending' as const,
    })),
};
