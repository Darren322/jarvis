import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, test } from 'vitest';
import type { Stats } from '../../src/link/protocol';
import { Dashboard } from '../../src/panels/Dashboard';
import { allValues, meterFillOf } from './dashboardQueries';

const READING: Stats = {
  cpuPercent: 12.5,
  temperatureC: 55.3,
  ramUsedBytes: 2_469_606_195,
  ramTotalBytes: 8_464_105_472,
  storage: { availableBytes: 207_517_036_544, totalBytes: 251_416_899_584 },
  llama: 'up',
  bge: 'down',
};

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement('div');
  root = createRoot(container);
});

afterEach(async () => {
  await act(async () => root.unmount());
});

async function show(reading: Stats | null): Promise<Record<string, string>> {
  await act(async () => root.render(<Dashboard reading={reading} />));
  return allValues(container);
}

test('CPU, temperature, RAM, storage, llama and BGE all render from a reading', async () => {
  expect(await show(READING)).toEqual({
    CPU: '13%',
    Temperature: '55°C',
    Memory: '2.3 / 7.9 GB',
    Storage: '44 / 251 GB',
    llama: 'Up',
    BGE: 'Down',
  });
});

test('a missing temperature shows "Unavailable", never 0', async () => {
  const values = await show({ ...READING, temperatureC: null });
  expect(values.Temperature).toBe('Unavailable');
});

test('a missing storage shows "Unavailable", never 0', async () => {
  const values = await show({ ...READING, storage: null });
  expect(values.Storage).toBe('Unavailable');
});

test('before the first reading every value shows the waiting mark, never 0 or "Unavailable"', async () => {
  const values = await show(null);
  expect(values).toEqual({ CPU: '—', Temperature: '—', Memory: '—', Storage: '—', llama: '—', BGE: '—' });
});

test('the widest values render whole', async () => {
  const values = await show({
    ...READING,
    cpuPercent: 100,
    temperatureC: 99.6,
    ramUsedBytes: READING.ramTotalBytes,
    storage: { availableBytes: 0, totalBytes: 9_999_000_000_000 },
  });
  expect([values.CPU, values.Temperature, values.Memory, values.Storage]).toEqual([
    '100%',
    '100°C',
    '7.9 / 7.9 GB',
    '9999 / 9999 GB',
  ]);
});

test('a service shows its health as a word as well as a marker', async () => {
  await show(READING);
  const services = [...container.querySelectorAll('.dashboard-service')].map((service) => [
    service.className,
    service.querySelector('.dashboard-marker') !== null,
    service.querySelector('dd')?.textContent,
  ]);
  expect(services).toEqual([
    ['dashboard-service dashboard-service-up', true, 'Up'],
    ['dashboard-service dashboard-service-down', true, 'Down'],
  ]);
});

test('each meter fills to its share of the scale, and is empty before the first reading', async () => {
  const fills = () =>
    [...container.querySelectorAll<HTMLElement>('.dashboard-meter > span')].map((fill) =>
      fill.style.getPropertyValue('--fill'),
    );
  await show({
    ...READING,
    cpuPercent: 50,
    temperatureC: 85,
    ramUsedBytes: READING.ramTotalBytes / 4,
    storage: { availableBytes: 750, totalBytes: 1000 },
  });
  expect(fills()).toEqual(['0.5', '1', '0.25', '0.25']);
  await show(null);
  expect(fills()).toEqual(['0', '0', '0', '0']);
});

test.each([
  // [available, total, fill]
  [1000, 1000, '0'],
  [0, 1000, '1'],
  [1001, 1000, '0'],
])('the storage meter for %d available of %d is %s full', async (availableBytes, totalBytes, fill) => {
  await show({ ...READING, storage: { availableBytes, totalBytes } });
  expect(meterFillOf(container, 'Storage')).toBe(fill);
});

test('the storage meter is empty when storage is missing', async () => {
  await show({ ...READING, storage: null });
  expect(meterFillOf(container, 'Storage')).toBe('0');
});
