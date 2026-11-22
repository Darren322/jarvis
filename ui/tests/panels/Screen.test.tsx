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
const SHOW_CAMERA = '{"type":"show_panel","panel":"camera"}';
const HIDE_PANEL = '{"type":"hide_panel"}';
const SHOW_BUILT =
  '{"type":"show_built_panel","title":"Pi","parts":[{"kind":"gauge","source":"cpu"},{"kind":"card","title":"Note","text":"<b>bold</b>"}]}';
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

function builtPanel(): HTMLElement | null {
  return container.querySelector<HTMLElement>('section[aria-label="Built panel"]');
}

function cameraPanel(): HTMLElement | null {
  return container.querySelector<HTMLElement>('section[aria-label="Camera"]');
}

/** The camera's picture, failing the test if it is not shown. */
function picture(): HTMLImageElement {
  const image = cameraPanel()?.querySelector('img');
  if (image === null || image === undefined) {
    throw new Error('no camera picture');
  }
  return image;
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

describe('the camera Panel', () => {
  test('ShowPanel camera shows the picture from the camera URL with the Orb in its corner', async () => {
    const { link, receive } = await mountScreen();
    await receive(SHOW_CAMERA);
    expect(orbBox()).toEqual(CORNER);
    expect(panel()).toBeNull();
    const source = new URL(picture().getAttribute('src') ?? '');
    expect(`${source.origin}${source.pathname}`).toBe(link.cameraUrl);
    expect(source.searchParams.get('showing')).not.toBeNull();
    expect(picture().getAttribute('alt')).toBe('Camera');
  });

  test('showing the camera again asks for a new picture, so it is never the last still frame', async () => {
    const { receive } = await mountScreen();
    await receive(SHOW_CAMERA);
    const first = picture().getAttribute('src');
    await receive(HIDE_PANEL);
    await receive(SHOW_CAMERA);
    expect(picture().getAttribute('src')).not.toBe(first);
  });

  test('a render while the camera shows keeps the same picture request', async () => {
    const { receive } = await mountScreen();
    await receive(SHOW_CAMERA);
    const first = picture().getAttribute('src');
    await receive(SHOW_CAMERA);
    expect(picture().getAttribute('src')).toBe(first);
  });

  test('the Orb is drawn after the camera Panel, so it stays on top', async () => {
    const { receive } = await mountScreen();
    await receive(SHOW_CAMERA);
    const order = [...container.children].map((element) => element.getAttribute('aria-label') ?? element.className);
    expect(order).toEqual(['panel-outside', 'Camera', 'Jarvis orb: tap to talk']);
  });

  test('the stats replace the camera and the camera replaces the stats', async () => {
    const { receive } = await mountScreen();
    await receive(SHOW_STATS, STATS);
    await receive(SHOW_CAMERA);
    expect(panel()).toBeNull();
    expect(cameraPanel()).not.toBeNull();
    await receive(SHOW_STATS);
    expect(cameraPanel()).toBeNull();
    expect(allValues(container).CPU).toBe('—');
  });

  test('HidePanel removes the picture, which ends its request, and gives the Orb the window back', async () => {
    const { receive } = await mountScreen();
    await receive(SHOW_CAMERA);
    await receive(HIDE_PANEL);
    expect(cameraPanel()).toBeNull();
    expect(container.querySelector('img')).toBeNull();
    expect(orbBox()).toEqual(FULL());
  });

  test('a tap outside the camera Panel sends one PanelClosed and hides it', async () => {
    const { socket, receive } = await mountScreen();
    await receive(SHOW_CAMERA);
    const outside = container.querySelector('.panel-outside');
    if (outside === null) {
      throw new Error('nothing catches taps outside the Panel');
    }
    await pointerDown(outside);
    expect(socket.sent).toEqual([PANEL_CLOSED]);
    expect(cameraPanel()).toBeNull();
  });

  test('a tap on the picture sends nothing and the camera stays', async () => {
    const { socket, receive } = await mountScreen();
    await receive(SHOW_CAMERA);
    await pointerDown(picture());
    expect(socket.sent).toEqual([]);
    expect(cameraPanel()).not.toBeNull();
  });

  test('a dropped connection hides the camera Panel', async () => {
    const { socket, receive } = await mountScreen();
    await receive(SHOW_CAMERA);
    await act(async () => socket.fail());
    expect(cameraPanel()).toBeNull();
    expect(orbBox()).toEqual(FULL());
  });

  test('state and level messages cause no React render while the camera shows', async () => {
    const { receive, commits } = await mountScreen();
    await receive(SHOW_CAMERA);
    const before = commits();
    await receive(
      '{"type":"state","state":"listening"}',
      ...Array.from({ length: 50 }, (_, index) => JSON.stringify({ type: 'level', level: index / 50, source: 'mic' })),
      STATS,
    );
    expect(commits()).toBe(before);
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

describe('the built Panel', () => {
  test('ShowBuiltPanel shows its parts with the Orb in its corner, drawn on top', async () => {
    const { receive } = await mountScreen();
    await receive(SHOW_BUILT);
    expect(orbBox()).toEqual(CORNER);
    expect(builtPanel()?.querySelector('h2')?.textContent).toBe('Pi');
    expect(builtPanel()?.querySelector('p')?.textContent).toBe('<b>bold</b>');
    expect(builtPanel()?.querySelector('b')).toBeNull();
    const order = [...container.children].map((element) => element.getAttribute('aria-label') ?? element.className);
    expect(order).toEqual(['panel-outside', 'Built panel', 'Jarvis orb: tap to talk']);
  });

  test('its gauge waits for a reading, then shows the latest one', async () => {
    const { receive } = await mountScreen();
    await receive(SHOW_BUILT);
    expect(allValues(container)).toEqual({ CPU: '—' });
    await receive(STATS);
    expect(allValues(container)).toEqual({ CPU: '13%' });
  });

  test('a reading re-renders the built Panel once', async () => {
    const { receive, commits } = await mountScreen();
    await receive(SHOW_BUILT);
    const before = commits();
    await receive(STATS);
    expect(commits()).toBe(before + 1);
  });

  test('a tap outside it sends one PanelClosed and hides it', async () => {
    const { socket, receive } = await mountScreen();
    await receive(SHOW_BUILT);
    const outside = container.querySelector('.panel-outside');
    if (outside === null) {
      throw new Error('nothing catches taps outside the Panel');
    }
    await pointerDown(outside);
    expect(socket.sent).toEqual([PANEL_CLOSED]);
    expect(builtPanel()).toBeNull();
    expect(orbBox()).toEqual(FULL());
  });

  test('a tap inside it sends nothing and it stays', async () => {
    const { socket, receive } = await mountScreen();
    await receive(SHOW_BUILT);
    const shown = builtPanel();
    if (shown === null) {
      throw new Error('the built Panel did not render');
    }
    await pointerDown(shown);
    expect(socket.sent).toEqual([]);
    expect(builtPanel()).not.toBeNull();
  });

  test('the stats and the camera replace it, and HidePanel closes it', async () => {
    const { receive } = await mountScreen();
    await receive(SHOW_BUILT, STATS);
    await receive(SHOW_STATS);
    expect(builtPanel()).toBeNull();
    expect(allValues(container).CPU).toBe('—');
    await receive(SHOW_BUILT);
    await receive(SHOW_CAMERA);
    expect(builtPanel()).toBeNull();
    expect(cameraPanel()).not.toBeNull();
    await receive(SHOW_BUILT);
    await receive(HIDE_PANEL);
    expect(builtPanel()).toBeNull();
    expect(orbBox()).toEqual(FULL());
  });

  test('a new built Panel replaces the one shown', async () => {
    const { receive } = await mountScreen();
    await receive(SHOW_BUILT);
    await receive('{"type":"show_built_panel","title":"Shopping","parts":[{"kind":"list","title":"To buy","items":["milk","eggs"]}]}');
    expect(builtPanel()?.querySelector('h2')?.textContent).toBe('Shopping');
    expect([...container.querySelectorAll('li')].map((item) => item.textContent)).toEqual(['milk', 'eggs']);
  });
});

describe('the timer strip', () => {
  const TIMERS =
    '{"type":"timers","timers":[{"id":3,"label":"Time to sleep","ends_at_ms":9007199254740991,"state":"pending"},{"id":4,"ends_at_ms":0,"state":"ended"}]}';
  const NO_TIMERS = '{"type":"timers","timers":[]}';

  function strip(): HTMLElement | null {
    return container.querySelector<HTMLElement>('section[aria-label="Timers"]');
  }

  function order(): string[] {
    return [...container.children].map((element) => element.getAttribute('aria-label') ?? element.className);
  }

  async function click(name: string) {
    const button = [...container.querySelectorAll('button')].find(
      (element) => (element.getAttribute('aria-label') ?? element.textContent) === name,
    );
    if (button === undefined) {
      throw new Error(`no button named ${name}`);
    }
    await act(async () => button.click());
  }

  test('it shows at the top right over the full-size Orb, drawn last so a tap reaches it', async () => {
    const { receive } = await mountScreen();
    await receive(TIMERS);
    expect(order()).toEqual(['Jarvis orb: tap to talk', 'Timers']);
    const { top, right, width } = strip()?.style ?? {};
    expect([top, right, width]).toEqual(['12px', '12px', '160px']);
    expect(orbBox()).toEqual(FULL());
  });

  test('with a Panel it stays, drawn last, and the Panel narrows to leave it room', async () => {
    const { receive } = await mountScreen();
    await receive(SHOW_STATS, TIMERS);
    expect(order()).toEqual(['panel-outside', 'Stats', 'Jarvis orb: tap to talk', 'Timers']);
    expect(panel()?.style.right).toBe('184px');
    expect(panel()?.classList.contains('panel-beside-timers')).toBe(true);
    expect(orbBox()).toEqual(CORNER);
    await receive(NO_TIMERS);
    expect(strip()).toBeNull();
    expect(panel()?.style.right).toBe('20px');
    expect(panel()?.classList.contains('panel-beside-timers')).toBe(false);
  });

  test('taps on the strip while a Panel shows send only the timer answers: the Panel stays open', async () => {
    const { socket, receive } = await mountScreen();
    await receive(SHOW_BUILT, TIMERS);
    const box = strip();
    if (box === null) {
      throw new Error('no strip');
    }
    await pointerDown(box);
    await click("Set 'Time to sleep'");
    await click('Timer Ended');
    expect(socket.sent).toEqual(['{"type":"confirm_timer","id":3}', '{"type":"cancel_timer","id":4}']);
    expect(builtPanel()).not.toBeNull();
  });

  test('a tap on the strip with no Panel is not a Tap on the Orb', async () => {
    const { socket, receive } = await mountScreen();
    await receive(TIMERS);
    const box = strip();
    if (box === null) {
      throw new Error('no strip');
    }
    await pointerDown(box);
    await click("Don't set 'Time to sleep'");
    expect(socket.sent).toEqual(['{"type":"reject_timer","id":3}']);
  });

  test('a dropped connection clears the strip until Jarvis sends the timers again', async () => {
    const { socket, receive } = await mountScreen();
    await receive(SHOW_STATS, TIMERS);
    await act(async () => socket.fail());
    expect(strip()).toBeNull();
  });

  test('a timers message is one React commit, and state and level messages cause none while the strip shows', async () => {
    const { receive, commits } = await mountScreen();
    await receive(SHOW_STATS, STATS);
    let before = commits();
    await receive(TIMERS);
    expect(commits()).toBe(before + 1);
    before = commits();
    await receive(
      '{"type":"state","state":"speaking"}',
      ...Array.from({ length: 30 }, (_, index) => JSON.stringify({ type: 'level', level: index / 30, source: 'speech' })),
    );
    expect(commits()).toBe(before);
  });
});
