import { expect, test } from 'vitest';
import {
  CORNER_ORB_INSET,
  CORNER_ORB_SIDE,
  orbPlacement,
  PANEL_INSETS,
  PANEL_LEFT,
  panelInsets,
  TIMER_STRIP_BOX,
  TIMER_STRIP_INSET,
} from '../../src/panels/layout';

const KIOSK = { width: 800, height: 480 };

test.each([
  { width: 800, height: 480 },
  { width: 1024, height: 768 },
])('with no Panel the Orb fills a %o window', (windowSize) => {
  expect(orbPlacement(windowSize, null)).toEqual({ size: windowSize, position: { left: 0, top: 0 }, round: false });
});

test('with the Dashboard shown the Orb is a 160 px circle 12 px from the top-left corner', () => {
  expect(orbPlacement(KIOSK, 'stats')).toEqual({
    size: { width: 160, height: 160 },
    position: { left: 12, top: 12 },
    round: true,
  });
});

test('the corner Orb sits left of the Panel, so a tap on it never lands on the Panel', () => {
  const { size, position } = orbPlacement(KIOSK, 'stats');
  expect(position.left + size.width).toBeLessThan(PANEL_LEFT);
  expect(PANEL_INSETS.left).toBe(PANEL_LEFT);
  expect(PANEL_LEFT).toBe(CORNER_ORB_INSET + CORNER_ORB_SIDE + 12);
});

/** A box as left, top, right and bottom edges in CSS pixels on the kiosk. */
interface Edges {
  readonly left: number;
  readonly top: number;
  readonly right: number;
  readonly bottom: number;
}

function overlaps(a: Edges, b: Edges): boolean {
  return a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom;
}

/** The strip's column, as tall as it may grow on the kiosk. */
const STRIP: Edges = {
  left: KIOSK.width - TIMER_STRIP_BOX.right - TIMER_STRIP_BOX.width,
  top: TIMER_STRIP_BOX.top,
  right: KIOSK.width - TIMER_STRIP_BOX.right,
  bottom: KIOSK.height - TIMER_STRIP_INSET,
};

test('the timer strip is a 160 px column 12 px from the top-right corner, mirroring the corner Orb', () => {
  expect(TIMER_STRIP_BOX).toEqual({ top: 12, right: 12, width: 160, maxHeight: 'calc(100vh - 24px)' });
  expect(STRIP).toEqual({ left: 628, top: 12, right: 788, bottom: 468 });
});

test('the timer strip never covers the corner Orb', () => {
  const { size, position } = orbPlacement(KIOSK, 'stats');
  const orb = { left: position.left, top: position.top, right: position.left + size.width, bottom: position.top + size.height };
  expect(overlaps(STRIP, orb)).toBe(false);
});

test.each([
  // [the strip has rows, the Panel's right inset, its size on the kiosk]
  [false, 20, [596, 440]],
  [true, 184, [432, 440]],
])('with timers %s the Panel has right inset %i and is %o px', (hasTimers, right, size) => {
  const insets = panelInsets(hasTimers);
  expect(insets).toEqual({ ...PANEL_INSETS, right });
  expect([KIOSK.width - insets.left - insets.right, KIOSK.height - insets.top - insets.bottom]).toEqual(size);
});

test('beside the strip the Panel keeps the Orb gap on both sides and never meets the strip', () => {
  const insets = panelInsets(true);
  const frame = { left: insets.left, top: insets.top, right: KIOSK.width - insets.right, bottom: KIOSK.height - insets.bottom };
  expect(overlaps(STRIP, frame)).toBe(false);
  expect(STRIP.left - frame.right).toBe(12);
  expect(panelInsets(false)).toBe(PANEL_INSETS);
});

test('on the 800x480 kiosk the Panel is 596 by 440 px and leaves a margin to tap outside it', () => {
  const width = KIOSK.width - PANEL_INSETS.left - PANEL_INSETS.right;
  const height = KIOSK.height - PANEL_INSETS.top - PANEL_INSETS.bottom;
  expect([width, height]).toEqual([596, 440]);
  expect(Math.min(PANEL_INSETS.top, PANEL_INSETS.right, PANEL_INSETS.bottom)).toBeGreaterThan(0);
});
