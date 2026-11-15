import { describe, expect, test } from 'vitest';
import {
  clampLevel,
  COLOR_KEYS,
  easeFactor,
  emptyLook,
  type Look,
  lerpLookInto,
  look,
  levelResponse,
  NUMBER_KEYS,
  VIEWS,
} from '../../src/orb/look';

const STATES = ['idle', 'listening', 'transcribing', 'thinking', 'speaking'] as const;
/** Raw levels from silence to a clipped microphone. */
const LEVELS = [0, 0.01, 0.05, 0.1, 0.5, 1];

function numbers(value: Look): number[] {
  return [...COLOR_KEYS.flatMap((key) => value[key]), ...NUMBER_KEYS.map((key) => value[key])];
}

describe('the state-to-look mapping', () => {
  test('each of the five states has its own look', () => {
    for (const level of [0, 0.08, 1]) {
      const looks = STATES.map((state) => JSON.stringify(look(state, level)));
      expect(new Set(looks).size).toBe(STATES.length);
    }
  });

  test('offline differs from every state', () => {
    const offline = JSON.stringify(look('offline', 0));
    for (const state of STATES) {
      expect(JSON.stringify(look(state, 0))).not.toBe(offline);
    }
  });

  test('offline is grey: every colour has equal red, green and blue', () => {
    const offline = look('offline', 0);
    for (const key of COLOR_KEYS) {
      const [r, g, b] = offline[key];
      expect([g, b]).toEqual([r, r]);
    }
  });

  test('the mapping is pure: the same input gives an equal, new look', () => {
    for (const view of VIEWS) {
      const first = look(view, 0.3);
      const second = look(view, 0.3);
      expect(second).toEqual(first);
      expect(second).not.toBe(first);
      expect(second.coreColor).not.toBe(first.coreColor);
    }
  });

  test('only listening and speaking follow the level', () => {
    for (const view of VIEWS) {
      const follows = JSON.stringify(look(view, 0)) !== JSON.stringify(look(view, 1));
      expect([view, follows]).toEqual([view, view === 'listening' || view === 'speaking']);
    }
  });

  test('every look is finite, with colours and strengths within 0 to 1', () => {
    const unit = (value: number) => value >= 0 && value <= 1;
    for (const view of VIEWS) {
      for (const level of [...LEVELS, -1, 7.5, Number.NaN, Infinity, -Infinity]) {
        const value = look(view, level);
        expect(numbers(value).every(Number.isFinite)).toBe(true);
        expect(COLOR_KEYS.flatMap((key) => value[key]).every(unit)).toBe(true);
        const strengths = [
          value.brightness,
          value.bandAlpha,
          value.hudAlpha,
          value.nodePulse,
          value.nodeJitter,
          value.coreGlow,
          value.rayStrength,
          value.background,
          value.blobSaturation,
        ];
        expect(strengths.every(unit)).toBe(true);
      }
    }
  });
});

describe('the level', () => {
  test.each([
    // [raw, clamped]
    [-0.01, 0],
    [0, 0],
    [1, 1],
    [1.01, 1],
    [Number.NaN, 0],
  ])('%d clamps to %d', (raw, clamped) => {
    expect(clampLevel(raw)).toBe(clamped);
  });

  test.each([
    // [out of range, the in-range level it must look like]
    [-0.01, 0],
    [1.01, 1],
  ])('a level of %d looks the same as %d in every view', (outside, inside) => {
    for (const view of VIEWS) {
      expect(look(view, outside)).toEqual(look(view, inside));
    }
  });

  test('the response is 0 at silence, rises with the level and is nearly 1 at full', () => {
    expect(levelResponse(0)).toBe(0);
    for (let index = 1; index < LEVELS.length; index += 1) {
      expect(levelResponse(LEVELS[index] ?? 0)).toBeGreaterThan(levelResponse(LEVELS[index - 1] ?? 0));
    }
    expect(levelResponse(1)).toBeCloseTo(1, 6);
  });
});

describe('easing', () => {
  test('a zero, negative or NaN step moves nothing', () => {
    expect([easeFactor(0), easeFactor(-0.1), easeFactor(Number.NaN)]).toEqual([0, 0, 0]);
  });

  test('a third of a second covers about 95% of a change', () => {
    expect(easeFactor(1 / 3)).toBeCloseTo(0.95, 2);
  });

  test('a blend is exact at both ends', () => {
    const from = look('idle', 0);
    const to = look('thinking', 0);
    expect(lerpLookInto(emptyLook(), from, to, 0)).toEqual(from);
    expect(lerpLookInto(emptyLook(), from, to, 1)).toEqual(to);
  });

  test('a blend may write into its own source', () => {
    const current = look('idle', 0);
    const to = look('speaking', 1);
    for (let step = 0; step < 200; step += 1) {
      lerpLookInto(current, current, to, easeFactor(1 / 60));
    }
    numbers(current).forEach((value, index) => expect(value).toBeCloseTo(numbers(to)[index] ?? Number.NaN, 5));
  });
});
