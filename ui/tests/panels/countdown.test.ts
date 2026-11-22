import { expect, test } from 'vitest';
import { formatCountdown, timerName } from '../../src/panels/countdown';

test.each([
  // [milliseconds left, shown]
  [3_600_000, '1:00:00'],
  [3_599_999, '1:00:00'],
  [3_599_000, '59:59'],
  [3_600_001, '1:00:01'],
  [600_000, '10:00'],
  [61_000, '1:01'],
  [59_999, '1:00'],
  [10_000, '0:10'],
  [1_000, '0:01'],
  [1, '0:01'],
  [0, '0:00'],
  [-1, '0:00'],
  [-90_000, '0:00'],
  [86_399_000, '23:59:59'],
  [36_000_000, '10:00:00'],
])('%i ms left reads %s', (remainingMs, shown) => {
  expect(formatCountdown(remainingMs)).toBe(shown);
});

test.each([
  // [label, name]
  ['Time to sleep', 'Time to sleep'],
  [null, 'Timer'],
  ['', 'Timer'],
  ['   ', 'Timer'],
  ['<b>bold</b>', '<b>bold</b>'],
])('the label %j is shown as %j', (label, name) => {
  expect(timerName(label)).toBe(name);
});
