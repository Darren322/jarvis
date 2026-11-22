import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest';
import type { ShownTimers } from '../../src/link/link';
import type { Timer } from '../../src/link/protocol';
import { Timers } from '../../src/panels/Timers';
import { cssSource } from './cssSource';

const timersCss = cssSource('panels/timers.css');

/** The UI's clock when each test starts: 2026-11-22 22:00 UTC, as Unix milliseconds. */
const T0 = Date.UTC(2026, 10, 22, 22, 0, 0);
const SECOND = 1000;
const MINUTE = 60 * SECOND;
const HOUR = 60 * MINUTE;

let container: HTMLDivElement;
let root: Root;
let sent: string[];

beforeEach(() => {
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  vi.useFakeTimers();
  vi.setSystemTime(T0);
  container = document.createElement('div');
  document.body.append(container);
  root = createRoot(container);
  sent = [];
});

afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
  vi.useRealTimers();
});

const running = (id: number, label: string | null, endsAtMs: number): Timer => ({ id, label, endsAtMs, state: 'running' });
const pending = (id: number, label: string | null): Timer => ({ id, label, endsAtMs: T0 + HOUR, state: 'pending' });
const ended = (id: number, label: string | null): Timer => ({ id, label, endsAtMs: T0 - MINUTE, state: 'ended' });

/** Shows the strip with `timers`, as if the message arrived at `receivedAtMs` (default: now). */
async function show(timers: readonly Timer[], receivedAtMs = Date.now()) {
  const shown: ShownTimers = { timers, receivedAtMs };
  await act(async () =>
    root.render(
      <Timers
        shown={shown}
        onConfirm={(id) => sent.push(`confirm_timer ${id}`)}
        onReject={(id) => sent.push(`reject_timer ${id}`)}
        onCancel={(id) => sent.push(`cancel_timer ${id}`)}
      />,
    ),
  );
}

async function wait(ms: number) {
  await act(async () => vi.advanceTimersByTime(ms));
}

/** A button's accessible name: its `aria-label`, else its text. */
function nameOf(button: HTMLButtonElement): string {
  return button.getAttribute('aria-label') ?? button.textContent ?? '';
}

/** Each row as Darren reads it: its name, its value (time left or "Ended") and its buttons' names. */
function rows() {
  return [...container.querySelectorAll('li')].map((row) => ({
    name: row.querySelector('.timer-name')?.textContent ?? null,
    value: row.querySelector('.timer-value')?.textContent ?? null,
    buttons: [...row.querySelectorAll('button')].map(nameOf),
  }));
}

function button(name: string): HTMLButtonElement {
  const found = [...container.querySelectorAll('button')].find((element) => nameOf(element) === name);
  if (found === undefined) {
    throw new Error(`no button named ${name}`);
  }
  return found;
}

async function tap(name: string) {
  await act(async () => button(name).click());
}

/** The time left on the only running row. */
function timeLeft(): string | null | undefined {
  return rows().find((row) => row.value !== 'Ended')?.value;
}

describe('rows', () => {
  test('no timers draw nothing at all', async () => {
    await show([]);
    expect(container.childElementCount).toBe(0);
  });

  test('one timer is one row, named by its label', async () => {
    await show([running(3, 'Time to sleep', T0 + 25 * MINUTE)]);
    expect(container.querySelector('section')?.getAttribute('aria-label')).toBe('Timers');
    expect(rows()).toEqual([{ name: 'Time to sleep', value: '25:00', buttons: ['Time to sleep 25:00'] }]);
  });

  test('five timers are five rows in the order Jarvis sent them, each with its own controls', async () => {
    await show([
      pending(1, 'Time to sleep'),
      running(2, 'Time to do work', T0 + 25 * MINUTE),
      running(3, null, T0 + 90 * MINUTE),
      ended(4, 'Pasta'),
      running(5, '', T0 + 10 * SECOND),
    ]);
    expect(rows()).toEqual([
      { name: 'Time to sleep', value: null, buttons: ["Set 'Time to sleep'", "Don't set 'Time to sleep'"] },
      { name: 'Time to do work', value: '25:00', buttons: ['Time to do work 25:00'] },
      { name: 'Timer', value: '1:30:00', buttons: ['Timer 1:30:00'] },
      { name: 'Pasta', value: 'Ended', buttons: ['Pasta Ended'] },
      { name: 'Timer', value: '0:10', buttons: ['Timer 0:10'] },
    ]);
  });

  test('a pending row shows Set and Don’t and no time', async () => {
    await show([pending(1, 'Time to sleep')]);
    expect([...container.querySelectorAll('button')].map((element) => element.textContent)).toEqual(['Set', "Don't"]);
    expect(container.querySelector('.timer-value')).toBeNull();
  });

  test('every control is a real button with a name', async () => {
    await show([pending(1, 'Time to sleep'), running(2, 'Work', T0 + MINUTE), ended(3, 'Tea')]);
    for (const element of container.querySelectorAll('button')) {
      expect(element.getAttribute('type')).toBe('button');
      expect(nameOf(element).trim()).not.toBe('');
    }
    await tap('Work 1:00');
    expect([...container.querySelectorAll('button')].map(nameOf)).toContain('Keep');
  });
});

describe('the countdown', () => {
  test.each([
    // [time left when the message arrives, shown]
    [HOUR - SECOND, '59:59'],
    [HOUR, '1:00:00'],
    [0, '0:00'],
    [-5 * SECOND, '0:00'],
  ])('%i ms left reads %s', async (left, shown) => {
    await show([running(1, 'Tea', T0 + left)]);
    expect(timeLeft()).toBe(shown);
  });

  test('it counts down once a second with the UI clock and stops at 0:00, never negative', async () => {
    await show([running(1, 'Tea', T0 + HOUR + SECOND)]);
    expect(timeLeft()).toBe('1:00:01');
    await wait(SECOND);
    expect(timeLeft()).toBe('1:00:00');
    await wait(SECOND);
    expect(timeLeft()).toBe('59:59');
    await wait(HOUR - SECOND);
    expect(timeLeft()).toBe('0:00');
    await wait(10 * SECOND);
    expect(timeLeft()).toBe('0:00');
  });

  test('a new message shows its time at once, without waiting for the next tick', async () => {
    await show([running(1, 'Tea', T0 + 10 * MINUTE)]);
    await wait(SECOND + 900);
    // The strip last read the clock 900 ms ago; a row from that reading would show 25:01.
    await show([running(1, 'Tea', T0 + 10 * MINUTE), running(2, 'Work', Date.now() + 25 * MINUTE)]);
    expect(rows().map((row) => row.value)).toEqual(['9:59', '25:00']);
  });

  test('the whole strip runs one interval, only while a row counts down, and clears it on unmount', async () => {
    await show([pending(1, 'Sleep'), ended(2, 'Tea')]);
    expect(vi.getTimerCount()).toBe(0);
    await show([running(1, 'Sleep', T0 + HOUR), running(2, 'Tea', T0 + MINUTE), running(3, 'Work', T0 + 2 * HOUR)]);
    expect(vi.getTimerCount()).toBe(1);
    await show([ended(2, 'Tea')]);
    expect(vi.getTimerCount()).toBe(0);
    await show([running(1, 'Sleep', T0 + HOUR)]);
    expect(vi.getTimerCount()).toBe(1);
    await act(async () => root.unmount());
    expect(vi.getTimerCount()).toBe(0);
    root = createRoot(container);
  });
});

describe('taps', () => {
  test('Set sends confirm_timer and Don’t sends reject_timer, each with its own id', async () => {
    await show([pending(7, 'Time to sleep'), pending(8, null)]);
    await tap("Set 'Time to sleep'");
    await tap("Don't set 'Timer'");
    expect(sent).toEqual(['confirm_timer 7', 'reject_timer 8']);
  });

  test('a tap on a running row asks first; Cancel sends cancel_timer with its id', async () => {
    await show([running(3, 'Time to sleep', T0 + HOUR), running(4, 'Work', T0 + MINUTE)]);
    await tap('Time to sleep 1:00:00');
    expect(sent).toEqual([]);
    const question = container.querySelector('[role="group"]');
    expect(question?.getAttribute('aria-label')).toBe("Cancel 'Time to sleep'?");
    expect(question?.textContent).toBe("Cancel 'Time to sleep'?CancelKeep");
    expect(document.activeElement).toBe(button('Keep'));
    await tap('Cancel');
    expect(sent).toEqual(['cancel_timer 3']);
    expect(container.querySelector('[role="group"]')).toBeNull();
  });

  test('Keep closes the question and sends nothing', async () => {
    await show([running(3, 'Time to sleep', T0 + HOUR)]);
    await tap('Time to sleep 1:00:00');
    await tap('Keep');
    expect(sent).toEqual([]);
    expect(container.querySelector('[role="group"]')).toBeNull();
    expect(rows()).toEqual([{ name: 'Time to sleep', value: '1:00:00', buttons: ['Time to sleep 1:00:00'] }]);
  });

  test('the question closes by itself when its timer ends or leaves the list', async () => {
    await show([running(3, 'Time to sleep', T0 + HOUR)]);
    await tap('Time to sleep 1:00:00');
    await show([ended(3, 'Time to sleep')]);
    expect(container.querySelector('[role="group"]')).toBeNull();
    expect(rows()[0]?.value).toBe('Ended');

    await show([running(4, 'Work', T0 + HOUR)]);
    await tap('Work 1:00:00');
    await show([running(5, 'Tea', T0 + HOUR)]);
    expect(container.querySelector('[role="group"]')).toBeNull();
    // Closed for good: the same id coming back shows its row, not the old question.
    await show([running(4, 'Work', T0 + HOUR), running(5, 'Tea', T0 + HOUR)]);
    expect(container.querySelector('[role="group"]')).toBeNull();
  });

  test('a tap on an ended row sends cancel_timer with its id at once, with no question', async () => {
    await show([ended(9, 'Pasta')]);
    await tap('Pasta Ended');
    expect(sent).toEqual(['cancel_timer 9']);
    expect(container.querySelector('[role="group"]')).toBeNull();
  });
});

describe('labels', () => {
  const LONG = 'A very long label '.repeat(7).slice(0, 120);

  test('a 120-character label is kept whole in the page, cut only by CSS, with its full text as the title', async () => {
    expect(LONG).toHaveLength(120);
    await show([running(1, LONG, T0 + MINUTE), pending(2, LONG), ended(3, LONG)]);
    for (const name of container.querySelectorAll('.timer-name')) {
      expect(name.textContent).toBe(LONG);
      expect(name.getAttribute('title')).toBe(LONG);
    }
    expect(timersCss).toMatch(/\.timer-name \{[^}]*text-overflow: ellipsis;[^}]*white-space: nowrap;/);
    await tap(`${LONG} 1:00`);
    expect(container.querySelector('.timer-question-text')?.textContent).toBe(`Cancel '${LONG}'?`);
    expect(timersCss).toMatch(/\.timer-question-text \{[^}]*-webkit-line-clamp: 2;[^}]*overflow-wrap: anywhere;/);
  });

  test('markup in a label is shown as text, never as HTML', async () => {
    await show([running(1, '<b>bold</b><img src=x>', T0 + MINUTE), pending(2, '<i>it</i>')]);
    expect(container.querySelector('b, img, i')).toBeNull();
    expect(rows().map((row) => row.name)).toEqual(['<b>bold</b><img src=x>', '<i>it</i>']);
  });
});

describe('the ended flash', () => {
  test('flashes only opacity, and not at all when reduced motion is asked for', () => {
    expect(timersCss).toMatch(/\.timer-ended::after \{[^}]*animation: timer-flash /);
    expect(timersCss).toMatch(/@keyframes timer-flash \{\s*to \{\s*opacity: [\d.]+;\s*\}\s*\}/);
    expect(timersCss).toMatch(
      /@media \(prefers-reduced-motion: reduce\) \{\s*\.timer-ended::after \{\s*animation: none;\s*\}\s*\}/,
    );
  });
});
