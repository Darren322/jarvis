import { expect, test } from 'vitest';

// The source text of every module in the Orb and the link, read at build time.
const SOURCES = import.meta.glob(['../../src/orb/**/*.{ts,tsx}', '../../src/link/**/*.{ts,tsx}'], {
  query: '?raw',
  import: 'default',
  eager: true,
}) as Record<string, string>;

test('the sources were found', () => {
  expect(Object.keys(SOURCES).length).toBeGreaterThanOrEqual(8);
});

test('no fetch, no XMLHttpRequest and no setInterval in the Orb or the link', () => {
  for (const [path, text] of Object.entries(SOURCES)) {
    expect([path, /\bfetch\s*\(|XMLHttpRequest|setInterval/.test(text)]).toEqual([path, false]);
  }
});

test('the only timer is the reconnect timer in the link', () => {
  const timers = Object.entries(SOURCES).flatMap(([path, text]) =>
    [...text.matchAll(/\bsetTimeout\s*\(/g)].map(() => path),
  );
  expect(timers).toEqual(['../../src/link/link.ts']);
});
