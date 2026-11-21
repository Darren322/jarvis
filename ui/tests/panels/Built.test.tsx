import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, test } from 'vitest';
import type { BuiltPanel, Stats } from '../../src/link/protocol';
import { Built } from '../../src/panels/Built';
import { allValues } from './dashboardQueries';

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

async function show(panel: BuiltPanel, reading: Stats | null = READING) {
  await act(async () => root.render(<Built panel={panel} reading={reading} />));
}

/** Every gauge source, one part each: the most parts are 4, so two Panels. */
const SYSTEM: BuiltPanel = {
  title: 'Pi',
  parts: [
    { kind: 'gauge', source: 'cpu' },
    { kind: 'gauge', source: 'temperature' },
    { kind: 'gauge', source: 'ram' },
    { kind: 'gauge', source: 'storage' },
  ],
};
const MODELS: BuiltPanel = {
  title: 'Servers',
  parts: [
    { kind: 'gauge', source: 'llama' },
    { kind: 'gauge', source: 'bge' },
  ],
};

test('the title is the Panel heading', async () => {
  await show({ title: 'Shopping', parts: [{ kind: 'list', title: 'To buy', items: ['milk'] }] });
  expect(container.querySelector('h2')?.textContent).toBe('Shopping');
});

test('a card shows its title and text', async () => {
  await show({ title: 'Note', parts: [{ kind: 'card', title: 'Reminder', text: 'Water the plants' }] });
  const card = container.querySelector('article');
  expect(card?.querySelector('h3')?.textContent).toBe('Reminder');
  expect(card?.querySelector('p')?.textContent).toBe('Water the plants');
});

test('a list shows its title and every item in order', async () => {
  await show({ title: 'Shopping', parts: [{ kind: 'list', title: 'To buy', items: ['milk', 'eggs', 'bread'] }] });
  expect(container.querySelector('article h3')?.textContent).toBe('To buy');
  expect([...container.querySelectorAll('li')].map((item) => item.textContent)).toEqual(['milk', 'eggs', 'bread']);
});

test('gauges show the values of the latest reading as the Dashboard writes them', async () => {
  await show(SYSTEM);
  expect(allValues(container)).toEqual({
    CPU: '13%',
    Temperature: '55°C',
    Memory: '2.3 / 7.9 GB',
    Storage: '44 / 251 GB',
  });
  await show(MODELS);
  expect(allValues(container)).toEqual({ llama: 'Up', BGE: 'Down' });
});

test('a newer reading changes the gauges', async () => {
  await show(SYSTEM);
  await show(SYSTEM, { ...READING, cpuPercent: 80 });
  expect(allValues(container).CPU).toBe('80%');
});

test('a missing temperature or storage shows "Unavailable", never 0', async () => {
  await show(SYSTEM, { ...READING, temperatureC: null, storage: null });
  const values = allValues(container);
  expect([values.Temperature, values.Storage]).toEqual(['Unavailable', 'Unavailable']);
});

test('before the first reading every gauge shows the waiting mark', async () => {
  await show(SYSTEM, null);
  expect(Object.values(allValues(container))).toEqual(['—', '—', '—', '—']);
});

test('model text is shown as text: markup in a card or list stays literal', async () => {
  await show({
    title: '<i>Title</i>',
    parts: [
      { kind: 'card', title: '<script>x</script>', text: '<b>bold</b> & <img src=x onerror=alert(1)>' },
      { kind: 'list', title: 'List', items: ['<u>under</u>'] },
    ],
  });
  expect(container.querySelector('p')?.textContent).toBe('<b>bold</b> & <img src=x onerror=alert(1)>');
  expect(container.querySelector('li')?.textContent).toBe('<u>under</u>');
  expect(container.querySelector('h2')?.textContent).toBe('<i>Title</i>');
  expect(container.querySelector('b, i, u, img, script')).toBeNull();
});

test.each([1, 2, 3, 4])('%i parts are laid out in their own grid', async (count) => {
  await show({ title: 'Pi', parts: SYSTEM.parts.slice(0, count) });
  const grid = container.querySelector('.built-parts');
  expect(grid?.classList.contains(`built-parts-${count}`)).toBe(true);
  expect(grid?.children.length).toBe(count);
});
