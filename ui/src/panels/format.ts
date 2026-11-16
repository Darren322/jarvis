/**
 * How the Dashboard writes a Stats reading. Pure functions only, so each rule
 * is tested once in a table.
 *
 * A value is a `Readout`: the figure, drawn large, and the unit after it, drawn
 * small; together they read as one value ("13%").
 */

import type { ServiceHealth } from '../link/protocol';

/** A value as the Dashboard draws it: `{ figure: '13', unit: '%' }` reads "13%". */
export interface Readout {
  readonly figure: string;
  readonly unit: string;
}

/** Shown in place of every value until the first reading arrives. Never 0 and never `UNAVAILABLE`. */
export const WAITING = '—';

/** Shown for a temperature the Pi could not read. Never 0. */
export const UNAVAILABLE = 'Unavailable';

/** Bytes in one GB as the Dashboard counts it: 2^30, as `free -h` does (an 8 GB Pi reads about 7.9). */
export const BYTES_PER_GB = 2 ** 30;

/**
 * The temperature at which the temperature meter is full, in degrees C: the
 * Pi 5 throttles its CPU at 85 °C.
 */
export const TEMPERATURE_METER_FULL_C = 85;

/** The readout of every value before the first reading. */
export const WAITING_READOUT: Readout = { figure: WAITING, unit: '' };

/** A load as a whole percent: `12.5` reads "13%". */
export function formatPercent(percent: number): Readout {
  return { figure: `${Math.round(percent)}`, unit: '%' };
}

/** Whole degrees C, or `UNAVAILABLE` for a missing reading. */
export function formatCelsius(celsius: number | null): Readout {
  if (celsius === null) {
    return { figure: UNAVAILABLE, unit: '' };
  }
  return { figure: `${Math.round(celsius)}`, unit: '°C' };
}

/** Bytes as GB (2^30 bytes) to one decimal place, without the unit: `2^30` gives `"1.0"`. */
export function formatGb(bytes: number): string {
  return (bytes / BYTES_PER_GB).toFixed(1);
}

/** RAM as "used / total GB", each to one decimal place; the used figure is the one drawn large. */
export function formatRam(usedBytes: number, totalBytes: number): Readout {
  return { figure: formatGb(usedBytes), unit: ` / ${formatGb(totalBytes)} GB` };
}

/** A service's health as a word, so it never relies on colour alone. */
export function formatHealth(health: ServiceHealth): string {
  return health === 'up' ? 'Up' : 'Down';
}

/** How full a meter is, 0 to 1: `value / full`, clamped. A `full` of 0 or less gives 0. */
export function meterFraction(value: number, full: number): number {
  if (!(full > 0)) {
    return 0;
  }
  return Math.min(1, Math.max(0, value / full));
}
