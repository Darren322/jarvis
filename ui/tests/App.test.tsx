import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, expect, test, vi } from 'vitest';
import { App } from '../src/App';

/** A WebSocket that never connects, so the test opens no real localhost socket. */
class SilentWebSocket extends EventTarget {
  readonly readyState = 0;
  onopen: (() => void) | null = null;
  onmessage: (() => void) | null = null;
  onclose: (() => void) | null = null;
  onerror: (() => void) | null = null;
  send(): void {}
  close(): void {}
}

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

test('the app renders the Jarvis main landmark without a real connection', async () => {
  vi.stubGlobal('WebSocket', SilentWebSocket);
  // jsdom has no canvas; the renderer copes with a missing context.
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(null);
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  const container = document.createElement('div');
  const root = createRoot(container);
  await act(async () => {
    root.render(<App />);
  });
  const main = container.querySelector('main');
  expect(main?.getAttribute('aria-label')).toBe('Jarvis');
  await act(async () => {
    root.unmount();
  });
});

test('the app opens one connection, shared by the Orb and the Panel', async () => {
  const opened: SilentWebSocket[] = [];
  vi.stubGlobal(
    'WebSocket',
    class extends SilentWebSocket {
      constructor() {
        super();
        opened.push(this);
      }
    },
  );
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(null);
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  const container = document.createElement('div');
  const root = createRoot(container);
  await act(async () => {
    root.render(<App />);
  });
  expect(opened).toHaveLength(1);
  const socket = opened[0] as unknown as { onopen: () => void; onmessage: (event: MessageEvent) => void };
  await act(async () => {
    socket.onopen();
    socket.onmessage(new MessageEvent('message', { data: '{"type":"show_panel","panel":"stats"}' }));
  });
  expect(container.querySelector('section[aria-label="Stats"]')).not.toBeNull();
  const orb = container.querySelector<HTMLElement>('[aria-label="Jarvis orb: tap to talk"]');
  expect([orb?.style.width, orb?.style.height]).toEqual(['160px', '160px']);
  await act(async () => {
    root.unmount();
  });
});
