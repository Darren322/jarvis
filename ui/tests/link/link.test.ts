import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest';
import { JARVIS_PORT, JARVIS_URL, JarvisLink, RECONNECT_DELAY } from '../../src/link/link';
import { latest, socketFactory } from './fakeSocket';

const TAP = '{"type":"tap"}';

function connectedLink() {
  const factory = socketFactory();
  const link = new JarvisLink({ createSocket: factory.create });
  link.start();
  latest(factory.sockets).open();
  return { link, ...factory };
}

beforeEach(() => {
  vi.useFakeTimers();
});

afterEach(() => {
  vi.useRealTimers();
});

describe('connecting', () => {
  test('the link connects to 127.0.0.1 on the Rust default port 8765', () => {
    expect(JARVIS_PORT).toBe(8765);
    expect(JARVIS_URL).toBe('ws://127.0.0.1:8765/');
    const { create, sockets } = socketFactory();
    new JarvisLink({ createSocket: create }).start();
    expect(sockets.map((socket) => socket.url)).toEqual(['ws://127.0.0.1:8765/']);
  });

  test('the link is offline until the socket opens, then idle and silent', () => {
    const { create, sockets } = socketFactory();
    const link = new JarvisLink({ createSocket: create });
    expect(link.display).toEqual({ kind: 'offline' });
    link.start();
    expect(link.status).toBe('connecting');
    expect(link.display).toEqual({ kind: 'offline' });
    latest(sockets).open();
    expect(link.status).toBe('connected');
    expect(link.display).toEqual({ kind: 'online', state: 'idle', level: 0 });
  });

  test('starting twice opens one socket', () => {
    const { create, sockets } = socketFactory();
    const link = new JarvisLink({ createSocket: create });
    link.start();
    link.start();
    expect(sockets).toHaveLength(1);
  });
});

describe('messages', () => {
  test('a state message changes the display', () => {
    const { link, sockets } = connectedLink();
    latest(sockets).receive('{"type":"state","state":"thinking"}');
    expect(link.display).toEqual({ kind: 'online', state: 'thinking', level: 0 });
  });

  test.each([
    // [level sent, level shown]
    [-0.01, 0],
    [0, 0],
    [0.25, 0.25],
    [1, 1],
    [1.01, 1],
  ])('a mic level of %d while listening shows as %d', (sent, shown) => {
    const { link, sockets } = connectedLink();
    const socket = latest(sockets);
    socket.receive('{"type":"state","state":"listening"}');
    socket.receive(JSON.stringify({ type: 'level', level: sent, source: 'mic' }));
    expect(link.display).toEqual({ kind: 'online', state: 'listening', level: shown });
  });

  test.each([
    // [state, source, counts]
    ['listening', 'mic', true],
    ['listening', 'speech', false],
    ['speaking', 'speech', true],
    ['speaking', 'mic', false],
    ['idle', 'mic', false],
    ['thinking', 'speech', false],
  ] as const)('a %s orb %s a %s level', (state, source, counts) => {
    const { link, sockets } = connectedLink();
    const socket = latest(sockets);
    socket.receive(JSON.stringify({ type: 'state', state }));
    socket.receive(JSON.stringify({ type: 'level', level: 0.5, source }));
    expect(link.display).toEqual({ kind: 'online', state, level: counts ? 0.5 : 0 });
  });

  test('a state change resets the level', () => {
    const { link, sockets } = connectedLink();
    const socket = latest(sockets);
    socket.receive('{"type":"state","state":"speaking"}');
    socket.receive('{"type":"level","level":0.7,"source":"speech"}');
    socket.receive('{"type":"state","state":"idle"}');
    expect(link.display).toEqual({ kind: 'online', state: 'idle', level: 0 });
  });

  test('malformed and unknown messages are ignored and the connection stays open', () => {
    const { link, sockets } = connectedLink();
    const socket = latest(sockets);
    socket.receive('{"type":"state","state":"speaking"}');
    const before = link.display;
    for (const message of [
      'not json',
      '',
      '[]',
      'null',
      '42',
      '{}',
      '{"type":"show_panel","panel":"stats"}',
      '{"type":"hide_panel"}',
      '{"type":"stats","cpu":12}',
      '{"type":"state","state":"dancing"}',
      '{"type":"state"}',
      '{"type":"state","state":"idle","extra":1}',
      '{"type":"level","level":"loud","source":"speech"}',
      '{"type":"level","level":null,"source":"speech"}',
      '{"type":"level","level":0.5,"source":"radio"}',
      '{"type":"level","level":0.5}',
      '{"type":"tap"}',
      new ArrayBuffer(4),
    ]) {
      socket.receive(message);
    }
    expect(link.display).toBe(before);
    expect(link.status).toBe('connected');
    expect(socket.closeCalls).toBe(0);
    expect(sockets).toHaveLength(1);
    socket.receive('{"type":"level","level":0.4,"source":"speech"}');
    expect(link.display).toEqual({ kind: 'online', state: 'speaking', level: 0.4 });
  });
});

describe('taps', () => {
  test('a tap while connected sends exactly one Tap', () => {
    const { link, sockets } = connectedLink();
    expect(link.tap()).toBe(true);
    expect(latest(sockets).sent).toEqual([TAP]);
  });

  test('a tap while connecting sends nothing', () => {
    const { create, sockets } = socketFactory();
    const link = new JarvisLink({ createSocket: create });
    link.start();
    expect(link.tap()).toBe(false);
    expect(latest(sockets).sent).toEqual([]);
  });

  test('a tap while offline sends nothing', () => {
    const { link, sockets } = connectedLink();
    const socket = latest(sockets);
    socket.fail();
    expect(link.status).toBe('offline');
    expect(link.tap()).toBe(false);
    expect(socket.sent).toEqual([]);
  });

  test('a tap before start sends nothing', () => {
    expect(new JarvisLink({ createSocket: socketFactory().create }).tap()).toBe(false);
  });
});

describe('reconnecting', () => {
  test('after a disconnect the display is offline at once', () => {
    const { link, sockets } = connectedLink();
    latest(sockets).receive('{"type":"state","state":"thinking"}');
    latest(sockets).fail();
    expect(link.display).toEqual({ kind: 'offline' });
  });

  test('the next attempt starts after exactly RECONNECT_DELAY, and attempts continue', () => {
    const { sockets } = connectedLink();
    for (let attempt = 1; attempt <= 3; attempt += 1) {
      latest(sockets).fail();
      vi.advanceTimersByTime(RECONNECT_DELAY - 1);
      expect(sockets).toHaveLength(attempt);
      vi.advanceTimersByTime(1);
      expect(sockets).toHaveLength(attempt + 1);
    }
  });

  test('a connection that never opens is retried after RECONNECT_DELAY', () => {
    const { create, sockets } = socketFactory();
    const link = new JarvisLink({ createSocket: create });
    link.start();
    latest(sockets).fail();
    expect(link.status).toBe('offline');
    vi.advanceTimersByTime(RECONNECT_DELAY);
    expect(sockets).toHaveLength(2);
    latest(sockets).open();
    expect(link.status).toBe('connected');
  });

  test('a socket that cannot be created is retried after RECONNECT_DELAY', () => {
    let calls = 0;
    const link = new JarvisLink({
      createSocket: () => {
        calls += 1;
        throw new SyntaxError('bad url');
      },
    });
    link.start();
    expect(link.status).toBe('offline');
    vi.advanceTimersByTime(RECONNECT_DELAY);
    expect(calls).toBe(2);
    link.stop();
  });

  test('the reconnect timer is the only timer', () => {
    const { sockets } = connectedLink();
    expect(vi.getTimerCount()).toBe(0);
    latest(sockets).fail();
    expect(vi.getTimerCount()).toBe(1);
  });

  test('stop closes the socket and cancels the reconnect', () => {
    const { link, sockets } = connectedLink();
    const socket = latest(sockets);
    link.stop();
    expect(socket.closeCalls).toBe(1);
    expect(link.display).toEqual({ kind: 'offline' });
    expect(vi.getTimerCount()).toBe(0);
    socket.fail();
    vi.advanceTimersByTime(RECONNECT_DELAY * 3);
    expect(sockets).toHaveLength(1);
  });

  test('a stopped link can start again', () => {
    const { link, sockets } = connectedLink();
    link.stop();
    link.start();
    expect(sockets).toHaveLength(2);
  });
});
