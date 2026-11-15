import { describe, expect, test } from 'vitest';
import type { Display } from '../../src/link/display';
import type { OrbSource } from '../../src/link/link';
import { IDLE_FRAME_INTERVAL, MAX_STEP_SECONDS, OrbAnimator } from '../../src/orb/animator';
import { look } from '../../src/orb/look';
import { FakeRenderer } from './fakes';

const SIXTY_HZ = 1 / 60;

class Source implements OrbSource {
  constructor(public display: Display) {}
}

function online(state: 'idle' | 'listening' | 'transcribing' | 'thinking' | 'speaking', level = 0): Display {
  return { kind: 'online', state, level };
}

/** Drives `frames` display frames at 60 Hz and returns how many were drawn. */
function run(animator: OrbAnimator, frames: number, start = 0): number {
  let drawn = 0;
  for (let index = 0; index < frames; index += 1) {
    if (animator.frame(start + index * SIXTY_HZ, 800, 480, 1)) {
      drawn += 1;
    }
  }
  return drawn;
}

describe('frame rate', () => {
  test.each([
    // [display, frames drawn out of 60 at 60 Hz]
    [{ kind: 'offline' } as Display, 30],
    [online('idle'), 30],
    [online('listening'), 60],
    [online('transcribing'), 60],
    [online('thinking'), 60],
    [online('speaking'), 60],
  ])('%o draws %i of 60 display frames', (display, drawn) => {
    const animator = new OrbAnimator(new Source(display), new FakeRenderer());
    expect(run(animator, 60)).toBe(drawn);
  });

  test('the idle interval is a 30 fps frame', () => {
    expect(IDLE_FRAME_INTERVAL).toBeCloseTo(1 / 30, 9);
  });
});

describe('easing', () => {
  test('the first frame shows the target look at once', () => {
    const renderer = new FakeRenderer();
    new OrbAnimator(new Source(online('thinking')), renderer).frame(5, 800, 480, 1);
    expect(renderer.last.lookJson).toBe(JSON.stringify(look('thinking', 0)));
    expect(renderer.last.clock).toBe(0);
  });

  test('a change of state eases in instead of snapping', () => {
    const source = new Source(online('idle'));
    const renderer = new FakeRenderer();
    const animator = new OrbAnimator(source, renderer);
    run(animator, 2);
    source.display = online('thinking');
    run(animator, 1, 1);
    const target = JSON.stringify(look('thinking', 0));
    expect(renderer.last.lookJson).not.toBe(target);
    run(animator, 120, 2);
    const final = JSON.parse(renderer.last.lookJson) as Record<string, unknown>;
    expect(final.coreGlow).toBeCloseTo(look('thinking', 0).coreGlow, 5);
  });

  test('a stall advances the animation by at most MAX_STEP_SECONDS', () => {
    const renderer = new FakeRenderer();
    const animator = new OrbAnimator(new Source(online('thinking')), renderer);
    animator.frame(0, 800, 480, 1);
    animator.frame(30, 800, 480, 1);
    expect(renderer.last.clock).toBe(MAX_STEP_SECONDS);
  });

  test('a clock that goes backwards does not move the animation', () => {
    const renderer = new FakeRenderer();
    const animator = new OrbAnimator(new Source(online('thinking')), renderer);
    animator.frame(10, 800, 480, 1);
    animator.frame(9, 800, 480, 1);
    expect(renderer.last.clock).toBe(0);
  });
});

test('the frame carries the view, level and size', () => {
  const renderer = new FakeRenderer();
  new OrbAnimator(new Source(online('speaking', 0.4)), renderer).frame(0, 320, 200, 2);
  expect(renderer.last).toMatchObject({ view: 'speaking', level: 0.4, width: 320, height: 200 });
});
