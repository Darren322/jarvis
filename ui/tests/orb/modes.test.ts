import { describe, expect, test } from 'vitest';
import { bodyMode, bodySpeed, countScale, ENGINE_REFERENCE_SIZE, writeBodyOpts } from '../../src/orb/body/modes';
import { VIEWS } from '../../src/orb/look';

describe('the body', () => {
  test('every view maps to a thinking-orbs mode, and the five states use at least four different ones', () => {
    const modes = VIEWS.filter((view) => view !== 'offline').map(bodyMode);
    expect(new Set(modes).size).toBeGreaterThanOrEqual(4);
  });

  test('only listening and speaking follow the level', () => {
    for (const view of VIEWS) {
      const quiet = JSON.stringify(writeBodyOpts({}, view, 0, 1));
      const loud = JSON.stringify(writeBodyOpts({}, view, 1, 1));
      const follows = quiet !== loud || bodySpeed(view, 0) !== bodySpeed(view, 1);
      expect([view, follows]).toEqual([view, view === 'listening' || view === 'speaking']);
    }
  });

  test.each([
    // [level, the in-range level it must behave as]
    [-0.01, 0],
    [1.01, 1],
  ])('a level of %d drives the body like %d', (outside, inside) => {
    for (const view of VIEWS) {
      expect(writeBodyOpts({}, view, outside, 1)).toEqual(writeBodyOpts({}, view, inside, 1));
      expect(bodySpeed(view, outside)).toBe(bodySpeed(view, inside));
    }
  });

  test.each([
    // [body side in CSS px, count scale]
    [ENGINE_REFERENCE_SIZE * 2, 1],
    [ENGINE_REFERENCE_SIZE, 1],
    [ENGINE_REFERENCE_SIZE / 2, 0.5],
    [10, 0.3],
  ])('a %d px body draws %d of the full dot count', (side, scale) => {
    expect(countScale(side)).toBe(scale);
  });

  test('a smaller body asks the engine for fewer dots', () => {
    for (const view of VIEWS) {
      const full = writeBodyOpts({}, view, 0, 1);
      const small = writeBodyOpts({}, view, 0, 0.3);
      for (const [key, value] of Object.entries(small)) {
        expect([view, key, (value ?? 0) <= (full[key] ?? 0)]).toEqual([view, key, true]);
      }
    }
  });
});
