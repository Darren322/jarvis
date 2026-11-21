import { expect, test } from 'vitest';

// The source text of every module in the Orb, the link and the Panels, read at build time.
const SOURCES = import.meta.glob(
  ['../../src/orb/**/*.{ts,tsx}', '../../src/link/**/*.{ts,tsx}', '../../src/panels/**/*.{ts,tsx}'],
  {
    query: '?raw',
    import: 'default',
    eager: true,
  },
) as Record<string, string>;

const PANEL_SOURCES = Object.entries(SOURCES).filter(([path]) => path.startsWith('../../src/panels/'));

test('the sources were found', () => {
  expect(Object.keys(SOURCES).length).toBeGreaterThanOrEqual(14);
  expect(PANEL_SOURCES.length).toBeGreaterThanOrEqual(5);
});

test('no fetch, no XMLHttpRequest and no setInterval in the Orb, the link or the Panels', () => {
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

test('the Panels run no animation loop of their own', () => {
  for (const [path, text] of PANEL_SOURCES) {
    expect([path, /requestAnimationFrame/.test(text)]).toEqual([path, false]);
  }
});

test('nothing renders model text as markup: no innerHTML in the Orb, the link or the Panels', () => {
  for (const [path, text] of Object.entries(SOURCES)) {
    expect([path, /dangerouslySetInnerHTML|innerHTML|outerHTML|insertAdjacentHTML/.test(text)]).toEqual([path, false]);
  }
});
