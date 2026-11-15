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
});

test('the app renders the Jarvis main landmark without a real connection', async () => {
  vi.stubGlobal('WebSocket', SilentWebSocket);
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
