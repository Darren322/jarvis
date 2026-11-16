import { expect, test } from 'vitest';
import {
  CORNER_ORB_INSET,
  CORNER_ORB_SIDE,
  orbPlacement,
  PANEL_INSETS,
  PANEL_LEFT,
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

test('on the 800x480 kiosk the Panel is 596 by 440 px and leaves a margin to tap outside it', () => {
  const width = KIOSK.width - PANEL_INSETS.left - PANEL_INSETS.right;
  const height = KIOSK.height - PANEL_INSETS.top - PANEL_INSETS.bottom;
  expect([width, height]).toEqual([596, 440]);
  expect(Math.min(PANEL_INSETS.top, PANEL_INSETS.right, PANEL_INSETS.bottom)).toBeGreaterThan(0);
});
