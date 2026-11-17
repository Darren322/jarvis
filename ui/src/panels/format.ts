/**
 * How the Dashboard writes a Stats reading. Pure functions only, so each rule
 * is tested once in a table.
 *
 * A value is a `Readout`: the figure, drawn large, and the unit after it, drawn
 * small; together they read as one value ("13%").
 */

import type { ServiceHealth, StorageSpace } from '../link/protocol';

/** A value as the Dashboard draws it: `{ figure: '13', unit: '%' }` reads "13%". */
export interface Readout {
  readonly figure: string;
  readonly unit: string;
}

/** Shown in place of every value until the first reading arrives. Never 0 and never `UNAVAILABLE`. */
export const WAITING = '—';

/** Shown for a temperature or storage the Pi could not read. Never 0. */
export const UNAVAILABLE = 'Unavailable';

/**
 * Bytes in one GB of memory as the Dashboard counts it: 2^30, as `free -h`
 * does (an 8 GB Pi reads about 7.9). Not the storage unit: see `STORAGE_BYTES_PER_GB`.
 */
export const RAM_BYTES_PER_GB = 2 ** 30;

/**
 * Bytes in one GB of storage as the Dashboard counts it: 10^9, the unit drives
 * are sold in (the Pi's 256 GB drive reads 251). Not the memory unit: see `RAM_BYTES_PER_GB`.
 */
export const STORAGE_BYTES_PER_GB = 10 ** 9;

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

/** Memory bytes as GB (2^30 bytes) to one decimal place, without the unit: `2^30` gives `"1.0"`. */
export function formatRamGb(bytes: number): string {
  return (bytes / RAM_BYTES_PER_GB).toFixed(1);
}

/** RAM as "used / total GB", each to one decimal place; the used figure is the one drawn large. */
export function formatRam(usedBytes: number, totalBytes: number): Readout {
  return { figure: formatRamGb(usedBytes), unit: ` / ${formatRamGb(totalBytes)} GB` };
}

/**
 * The bytes in use: total minus available, so the blocks the filesystem keeps
 * in reserve count as used. Never below 0, even if available exceeds total.
 */
export function storageUsedBytes(space: StorageSpace): number {
  return Math.max(0, space.totalBytes - space.availableBytes);
}

/** Storage bytes as whole GB (10^9 bytes), without the unit: half a GB rounds up. */
export function formatStorageGb(bytes: number): string {
  return `${Math.round(bytes / STORAGE_BYTES_PER_GB)}`;
}

/**
 * Storage as "used / total GB" in whole GB, or `UNAVAILABLE` when the Pi could
 * not read its filesystem; the used figure is the one drawn large.
 */
export function formatStorage(space: StorageSpace | null): Readout {
  if (space === null) {
    return { figure: UNAVAILABLE, unit: '' };
  }
  return {
    figure: formatStorageGb(storageUsedBytes(space)),
    unit: ` / ${formatStorageGb(space.totalBytes)} GB`,
  };
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
