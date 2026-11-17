/**
 * Fixed Stats readings for the dev-only preview, so the Dashboard can be seen
 * and screenshotted without Jarvis.
 */

import type { Stats, StorageSpace } from '../link/protocol';

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
