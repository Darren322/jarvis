import { describe, expect, test } from 'vitest';
import {
  BYTES_PER_GB,
  formatCelsius,
  formatGb,
  formatHealth,
  formatPercent,
  formatRam,
  meterFraction,
  type Readout,
  UNAVAILABLE,
  WAITING,
  WAITING_READOUT,
} from '../../src/panels/format';

/** What Darren reads. */
const text = (readout: Readout) => `${readout.figure}${readout.unit}`;

/** The 8 GB Pi's total RAM, about what `free -h` calls 7.9 GiB. */
const PI_RAM = 8_464_105_472;

describe('percent', () => {
  test.each([
    [0, '0%'],
    [0.49, '0%'],
    [0.5, '1%'],
    [12.5, '13%'],
    [99.49, '99%'],
    [99.5, '100%'],
    [100, '100%'],
  ])('%d reads %s', (percent, shown) => {
    expect(text(formatPercent(percent))).toBe(shown);
  });

  test('the figure is the number and the unit is the sign', () => {
    expect(formatPercent(100)).toEqual({ figure: '100', unit: '%' });
  });
});

describe('temperature', () => {
  test.each([
    [null, UNAVAILABLE],
    [0, '0°C'],
    [-0.4, '0°C'],
    [-5, '-5°C'],
    [54.49, '54°C'],
    [54.5, '55°C'],
    [99.6, '100°C'],
  ])('%s reads %s', (celsius, shown) => {
    expect(text(formatCelsius(celsius))).toBe(shown);
  });

  test('a missing temperature reads "Unavailable", never 0', () => {
    expect(formatCelsius(null)).toEqual({ figure: 'Unavailable', unit: '' });
  });
});

describe('RAM', () => {
  test.each([
    [0, '0.0'],
    [BYTES_PER_GB, '1.0'],
    [1.049 * BYTES_PER_GB, '1.0'],
    [1.051 * BYTES_PER_GB, '1.1'],
    [PI_RAM, '7.9'],
  ])('%d bytes are %s GB', (bytes, shown) => {
    expect(formatGb(bytes)).toBe(shown);
  });

  test('a GB is 2^30 bytes', () => {
    expect(BYTES_PER_GB).toBe(1_073_741_824);
  });

  test.each([
    [0, PI_RAM, '0.0 / 7.9 GB'],
    [2_469_606_195, PI_RAM, '2.3 / 7.9 GB'],
    [PI_RAM, PI_RAM, '7.9 / 7.9 GB'],
  ])('%d used of %d reads %s', (used, total, shown) => {
    expect(text(formatRam(used, total))).toBe(shown);
  });

  test('the used figure is drawn large and the total goes with the unit', () => {
    expect(formatRam(PI_RAM, PI_RAM)).toEqual({ figure: '7.9', unit: ' / 7.9 GB' });
  });
});

test.each([
  ['up', 'Up'],
  ['down', 'Down'],
] as const)('a service that is %s reads %s', (health, shown) => {
  expect(formatHealth(health)).toBe(shown);
});

test('the waiting mark is neither 0 nor Unavailable', () => {
  expect(text(WAITING_READOUT)).toBe(WAITING);
  expect(WAITING).not.toMatch(/0/);
  expect(WAITING).not.toBe(UNAVAILABLE);
});

test.each([
  // [value, full, fraction]
  [-1, 100, 0],
  [0, 100, 0],
  [50, 100, 0.5],
  [100, 100, 1],
  [101, 100, 1],
  [5, 0, 0],
  [5, -1, 0],
  [5, Number.NaN, 0],
])('a meter for %d of %d is %d full', (value, full, fraction) => {
  expect(meterFraction(value, full)).toBe(fraction);
});
