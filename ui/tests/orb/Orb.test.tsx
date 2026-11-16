import { act, Profiler } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest';
import { JarvisLink } from '../../src/link/link';
import { look } from '../../src/orb/look';
import { Orb, type OrbSize } from '../../src/orb/Orb';
import type { CreateOrbRenderer } from '../../src/orb/renderer';
import { latest, socketFactory } from '../link/fakeSocket';
import { fakeRenderers, ManualFrames } from './fakes';

const FULL: OrbSize = { width: 800, height: 480 };
const CORNER: OrbSize = { width: 160, height: 160 };
const ORIGIN = { left: 0, top: 0 };
const TAP = '{"type":"tap"}';

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

/** Mounts an Orb on a link with a fake socket, a fake renderer and hand-stepped frames. */
async function mountOrb(options: { size?: OrbSize; createRenderer?: CreateOrbRenderer } = {}) {
  const sockets = socketFactory();
  const link = new JarvisLink({ createSocket: sockets.create });
  links.push(link);
  const renderers = fakeRenderers();
  const frames = new ManualFrames();
  let commits = 0;
  const render = async (size: OrbSize) => {
    await act(async () => {
      root.render(
        <Profiler id="orb" onRender={() => (commits += 1)}>
          <Orb
            source={link}
            onTap={link.tap}
            size={size}
            position={ORIGIN}
            createRenderer={options.createRenderer ?? renderers.create}
            frames={frames}
          />
        </Profiler>,
      );
    });
  };
  await render(options.size ?? FULL);
  link.start();
  return { link, sockets: sockets.sockets, renderers: renderers.made, frames, render, commits: () => commits };
}

function host(): HTMLElement {
  const element = container.querySelector<HTMLElement>('[role="button"]');
  if (element === null) {
    throw new Error('the Orb rendered no button');
  }
  return element;
}

function tapOrb(): void {
  host().dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }));
}

describe('re-renders', () => {
  test('100 level messages cause no React re-render, yet reach the frame loop', async () => {
    const { sockets, renderers, frames, commits } = await mountOrb();
    const socket = latest(sockets);
    await act(async () => {
      socket.open();
      socket.receive('{"type":"state","state":"speaking"}');
    });
    const before = commits();
    await act(async () => {
      for (let index = 1; index <= 100; index += 1) {
        socket.receive(JSON.stringify({ type: 'level', level: index / 100, source: 'speech' }));
      }
    });
    expect(commits()).toBe(before);
    frames.tick(0);
    expect(renderers[0]?.last).toMatchObject({ view: 'speaking', level: 1 });
  });

  test('state messages cause no React re-render either', async () => {
    const { sockets, commits } = await mountOrb();
    const socket = latest(sockets);
    const before = commits();
    await act(async () => {
      socket.open();
      for (const state of ['listening', 'transcribing', 'thinking', 'speaking', 'idle']) {
        socket.receive(JSON.stringify({ type: 'state', state }));
      }
    });
    expect(commits()).toBe(before);
  });
});

describe('the frame loop', () => {
  test('frames come from the frame scheduler, one request at a time', async () => {
    const { renderers, frames } = await mountOrb();
    expect(frames.pending).toBe(1);
    frames.tick(0);
    frames.tick(100);
    expect(renderers[0]?.frames).toHaveLength(2);
    expect(frames.pending).toBe(1);
  });

  test('unmounting stops the loop and disposes the renderer', async () => {
    const { renderers, frames } = await mountOrb();
    await act(async () => root.unmount());
    expect(frames.pending).toBe(0);
    expect(renderers[0]?.disposed).toBe(true);
    root = createRoot(container);
  });

  test('the renderer is given the Orb element to draw into', async () => {
    const { renderers } = await mountOrb();
    expect(renderers[0]?.host).toBe(host());
  });

  test('without a renderer the Orb draws nothing but still sends taps', async () => {
    const { sockets, frames } = await mountOrb({ createRenderer: () => null });
    expect(frames.pending).toBe(0);
    latest(sockets).open();
    tapOrb();
    expect(latest(sockets).sent).toEqual([TAP]);
  });
});

describe('offline', () => {
  test('before Jarvis answers, the Orb draws the offline look', async () => {
    const { renderers, frames } = await mountOrb();
    frames.tick(0);
    expect(renderers[0]?.last.view).toBe('offline');
    expect(renderers[0]?.last.lookJson).toBe(JSON.stringify(look('offline', 0)));
  });

  test('after a disconnect, the Orb heads back to the offline look', async () => {
    const { sockets, renderers, frames } = await mountOrb();
    latest(sockets).open();
    latest(sockets).receive('{"type":"state","state":"thinking"}');
    frames.tick(0);
    expect(renderers[0]?.last.view).toBe('thinking');
    latest(sockets).fail();
    frames.tick(100);
    expect(renderers[0]?.last.view).toBe('offline');
  });
});

describe('taps', () => {
  test('a tap while connected sends exactly one Tap', async () => {
    const { sockets } = await mountOrb();
    latest(sockets).open();
    tapOrb();
    expect(latest(sockets).sent).toEqual([TAP]);
  });

  test('a tap while connecting sends nothing', async () => {
    const { sockets } = await mountOrb();
    tapOrb();
    expect(latest(sockets).sent).toEqual([]);
  });

  test('a tap while offline sends nothing', async () => {
    vi.useFakeTimers();
    try {
      const { sockets } = await mountOrb();
      const first = latest(sockets);
      first.open();
      first.fail();
      tapOrb();
      expect(first.sent).toEqual([]);
      expect(sockets).toHaveLength(1);
    } finally {
      vi.useRealTimers();
    }
  });

  test('Enter and Space on the focused Orb count as a tap', async () => {
    const { sockets } = await mountOrb();
    latest(sockets).open();
    host().dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
    host().dispatchEvent(new KeyboardEvent('keydown', { key: ' ', bubbles: true }));
    host().dispatchEvent(new KeyboardEvent('keydown', { key: 'a', bubbles: true }));
    expect(latest(sockets).sent).toEqual([TAP, TAP]);
  });
});

describe('size and position', () => {
  test('the Orb draws at two sizes from the same state with the same look', async () => {
    const { sockets, renderers, frames, render } = await mountOrb({ size: FULL });
    latest(sockets).open();
    latest(sockets).receive('{"type":"state","state":"thinking"}');
    frames.tick(0);
    const full = renderers[0]?.last;
    await render(CORNER);
    frames.tick(100);
    const corner = renderers[0]?.last;
    expect([full?.width, full?.height]).toEqual([800, 480]);
    expect([corner?.width, corner?.height]).toEqual([160, 160]);
    expect(corner?.lookJson).toBe(full?.lookJson);
    expect(corner?.lookJson).toBe(JSON.stringify(look('thinking', 0)));
    // Resizing does not restart the loop or make a new renderer.
    expect(renderers).toHaveLength(1);
  });

  test('size and position place the Orb element', async () => {
    const sockets = socketFactory();
    const link = new JarvisLink({ createSocket: sockets.create });
    await act(async () => {
      root.render(
        <Orb
          source={link}
          onTap={link.tap}
          size={CORNER}
          position={{ left: 620, top: 300 }}
          createRenderer={() => null}
        />,
      );
    });
    const style = host().style;
    expect([style.left, style.top, style.width, style.height]).toEqual(['620px', '300px', '160px', '160px']);
    expect(style.borderRadius).toBe('');
  });

  test('a round Orb is clipped to the circle inside its area', async () => {
    const sockets = socketFactory();
    const link = new JarvisLink({ createSocket: sockets.create });
    await act(async () => {
      root.render(
        <Orb
          source={link}
          onTap={link.tap}
          size={CORNER}
          position={{ left: 12, top: 12 }}
          round
          createRenderer={() => null}
        />,
      );
    });
    const style = host().style;
    expect([style.borderRadius, style.overflow]).toEqual(['50%', 'hidden']);
  });
});
