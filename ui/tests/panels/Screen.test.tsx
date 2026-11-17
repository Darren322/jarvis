import { act, Profiler } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, test } from 'vitest';
import { JarvisLink } from '../../src/link/link';
import { Screen } from '../../src/panels/Screen';
import { latest, socketFactory } from '../link/fakeSocket';
import { fakeRenderers, ManualFrames } from '../orb/fakes';
import { allValues } from './dashboardQueries';

const TAP = '{"type":"tap"}';
const PANEL_CLOSED = '{"type":"panel_closed"}';
const SHOW_STATS = '{"type":"show_panel","panel":"stats"}';
const HIDE_PANEL = '{"type":"hide_panel"}';
const STATS =
  '{"type":"stats","cpu_percent":12.5,"ram_used_bytes":2469606195,"ram_total_bytes":8464105472,"llama":"up","bge":"down"}';

let container: HTMLDivElement;
let root: Root;
let links: JarvisLink[] = [];

beforeEach(() => {
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement('div');
  document.body.append(container);
  root = createRoot(container);
});

afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
  for (const link of links) {
    link.stop();
  }
  links = [];
});

/** Mounts the Screen on a connected link with a fake socket and a fake renderer; counts React commits. */
async function mountScreen() {
  const sockets = socketFactory();
  const link = new JarvisLink({ createSocket: sockets.create });
  links.push(link);
  let commits = 0;
  await act(async () => {
    root.render(
      <Profiler id="screen" onRender={() => (commits += 1)}>
        <Screen source={link} onTap={link.tap} createRenderer={fakeRenderers().create} frames={new ManualFrames()} />
      </Profiler>,
    );
  });
  link.start();
  const socket = latest(sockets.sockets);
  socket.open();
  /** Jarvis sends these lines, inside `act` so React catches up. */
  const receive = async (...lines: string[]) => {
    await act(async () => {
      for (const line of lines) {
        socket.receive(line);
      }
    });
  };
  return { link, socket, receive, commits: () => commits };
}

function orb(): HTMLElement {
  const element = container.querySelector<HTMLElement>('[aria-label="Jarvis orb: tap to talk"]');
  if (element === null) {
    throw new Error('no Orb');
  }
  return element;
}

/** The Orb's place as its props set it: left, top, width, height. */
function orbBox(): string[] {
  const { left, top, width, height } = orb().style;
  return [left, top, width, height];
}

function panel(): HTMLElement | null {
  return container.querySelector<HTMLElement>('section[aria-label="Stats"]');
}

async function pointerDown(target: Element) {
  await act(async () => {
    target.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }));
  });
}

const FULL = () => ['0px', '0px', `${window.innerWidth}px`, `${window.innerHeight}px`];
const CORNER = ['12px', '12px', '160px', '160px'];

describe('the Orb and the Panel', () => {
  test('with no Panel the Orb fills the window and nothing else shows', async () => {
    await mountScreen();
    expect(orbBox()).toEqual(FULL());
    expect(panel()).toBeNull();
    expect(container.querySelector('.panel-outside')).toBeNull();
  });

  test('ShowPanel stats moves the Orb to its corner size and HidePanel moves it back', async () => {
    const { receive } = await mountScreen();
    await receive(SHOW_STATS);
    expect(orbBox()).toEqual(CORNER);
    expect(panel()).not.toBeNull();
    await receive(HIDE_PANEL);
    expect(orbBox()).toEqual(FULL());
    expect(panel()).toBeNull();
  });

  test('the Orb is drawn after the Panel, so it stays on top', async () => {
    const { receive } = await mountScreen();
    await receive(SHOW_STATS);
    const order = [...container.children].map((element) => element.getAttribute('aria-label') ?? element.className);
    expect(order).toEqual(['panel-outside', 'Stats', 'Jarvis orb: tap to talk']);
  });

  test('the Dashboard waits for its first reading, then renders it', async () => {
    const { receive } = await mountScreen();
    await receive(SHOW_STATS);
    expect(allValues(container).CPU).toBe('—');
    await receive(STATS);
    expect(allValues(container)).toEqual({
      CPU: '13%',
      Temperature: 'Unavailable',
      Memory: '2.3 / 7.9 GB',
      Storage: 'Unavailable',
      llama: 'Up',
      BGE: 'Down',
    });
  });

  test('a dropped connection hides the Panel and gives the Orb the window back', async () => {
    const { socket, receive } = await mountScreen();
    await receive(SHOW_STATS, STATS);
    await act(async () => socket.fail());
    expect(panel()).toBeNull();
    expect(orbBox()).toEqual(FULL());
  });
});

describe('taps', () => {
  test('a tap outside the Panel sends one PanelClosed, hides the Panel and moves the Orb back', async () => {
    const { socket, receive } = await mountScreen();
    await receive(SHOW_STATS, STATS);
    const outside = container.querySelector('.panel-outside');
    if (outside === null) {
      throw new Error('nothing catches taps outside the Panel');
    }
    await pointerDown(outside);
    expect(socket.sent).toEqual([PANEL_CLOSED]);
    expect(panel()).toBeNull();
    expect(orbBox()).toEqual(FULL());
  });

  test('a tap inside the Panel sends nothing and the Panel stays', async () => {
    const { socket, receive } = await mountScreen();
    await receive(SHOW_STATS, STATS);
    const shown = panel();
    const value = container.querySelector('.dashboard-figure');
    if (shown === null || value === null) {
      throw new Error('the Dashboard did not render');
    }
    await pointerDown(shown);
    await pointerDown(value);
    expect(socket.sent).toEqual([]);
    expect(panel()).not.toBeNull();
  });

  test('a tap on the Orb sends one Tap and the Panel stays', async () => {
    const { socket, receive } = await mountScreen();
    await receive(SHOW_STATS);
    await pointerDown(orb());
    expect(socket.sent).toEqual([TAP]);
    expect(panel()).not.toBeNull();
    expect(orbBox()).toEqual(CORNER);
  });
});

describe('re-renders', () => {
  test('state and level messages cause no React render while the Dashboard shows', async () => {
    const { receive, commits } = await mountScreen();
    await receive(SHOW_STATS, STATS);
    const before = commits();
    await receive(
      '{"type":"state","state":"speaking"}',
      ...Array.from({ length: 100 }, (_, index) =>
        JSON.stringify({ type: 'level', level: (index + 1) / 100, source: 'speech' }),
      ),
      '{"type":"state","state":"idle"}',
    );
    expect(commits()).toBe(before);
  });

  test('a reading re-renders the Dashboard once', async () => {
    const { receive, commits } = await mountScreen();
    await receive(SHOW_STATS);
    const before = commits();
    await receive(STATS);
    expect(commits()).toBe(before + 1);
  });
});
