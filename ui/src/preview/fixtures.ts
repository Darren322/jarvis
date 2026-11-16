/**
 * Fixed Stats readings for the dev-only preview, so the Dashboard can be seen
 * and screenshotted without Jarvis.
 */

import type { Stats } from '../link/protocol';

/** About what `free -h` calls 7.9 GiB: the 8 GB Pi's total RAM. */
const PI_RAM_TOTAL_BYTES = 8_464_105_472;

const TYPICAL: Stats = {
  cpuPercent: 12.5,
  temperatureC: 55.3,
  ramUsedBytes: 2_469_606_195,
  ramTotalBytes: PI_RAM_TOTAL_BYTES,
  llama: 'up',
  bge: 'up',
};

/** Each fixture by its `reading` URL parameter; `waiting` is the Dashboard before its first reading. */
export const READINGS: Readonly<Record<string, Stats | null>> = {
  typical: TYPICAL,
  'no-temperature': { ...TYPICAL, temperatureC: null },
  waiting: null,
  'service-down': { ...TYPICAL, bge: 'down' },
  // The widest text each value can take on the 8 GB Pi.
  widest: {
    cpuPercent: 100,
    temperatureC: 99.6,
    ramUsedBytes: PI_RAM_TOTAL_BYTES,
    ramTotalBytes: PI_RAM_TOTAL_BYTES,
    llama: 'down',
    bge: 'down',
  },
};
