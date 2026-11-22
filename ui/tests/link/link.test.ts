import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest';
import { CAMERA_URL, cameraUrl, JARVIS_PORT, JARVIS_URL, JarvisLink, RECONNECT_DELAY } from '../../src/link/link';
import { latest, socketFactory } from './fakeSocket';

const TAP = '{"type":"tap"}';
const PANEL_CLOSED = '{"type":"panel_closed"}';
const SHOW_STATS = '{"type":"show_panel","panel":"stats"}';
const HIDE_PANEL = '{"type":"hide_panel"}';
const STATS =
  '{"type":"stats","cpu_percent":12.5,"temperature_c":55.0,"ram_used_bytes":2,"ram_total_bytes":8,"llama":"up","bge":"down"}';
const READING = { cpuPercent: 12.5, temperatureC: 55, ramUsedBytes: 2, ramTotalBytes: 8, storage: null, llama: 'up', bge: 'down' };

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

describe('the camera URL', () => {
  test.each([
    // [WebSocket URL, camera URL]
    ['ws://127.0.0.1:8765/', 'http://127.0.0.1:8765/camera'],
    ['ws://localhost:9000/', 'http://localhost:9000/camera'],
    ['ws://127.0.0.1:8765', 'http://127.0.0.1:8765/camera'],
    ['wss://jarvis.local:443/', 'https://jarvis.local/camera'],
  ])('%s gives %s: the same host and port as the WebSocket', (socketUrl, expected) => {
    expect(cameraUrl(socketUrl)).toBe(expected);
  });

  test('the default is the default WebSocket host and port', () => {
    expect(CAMERA_URL).toBe('http://127.0.0.1:8765/camera');
    expect(new JarvisLink().cameraUrl).toBe(CAMERA_URL);
  });

  test('a link to another WebSocket shows the camera from there', () => {
    expect(new JarvisLink({ url: 'ws://localhost:9000/' }).cameraUrl).toBe('http://localhost:9000/camera');
  });
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
      '{"type":"show_panel","panel":"weather"}',
      '{"type":"hide_panel","panel":"stats"}',
      '{"type":"stats","cpu":12}',
      STATS.replace('12.5', '100.01'),
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
    expect(link.shownPanel).toBeNull();
    expect(link.status).toBe('connected');
    expect(socket.closeCalls).toBe(0);
    expect(sockets).toHaveLength(1);
    socket.receive('{"type":"level","level":0.4,"source":"speech"}');
    expect(link.display).toEqual({ kind: 'online', state: 'speaking', level: 0.4 });
  });
});

describe('the Panel', () => {
  test('Panel messages reach the Panel and leave the display alone', () => {
    const { link, sockets } = connectedLink();
    const socket = latest(sockets);
    socket.receive('{"type":"state","state":"speaking"}');
    const display = link.display;
    expect([link.shownPanel, link.stats]).toEqual([null, null]);
    socket.receive(SHOW_STATS);
    expect([link.shownPanel, link.stats]).toEqual(['stats', null]);
    socket.receive(STATS);
    expect([link.shownPanel, link.stats]).toEqual(['stats', READING]);
    socket.receive(HIDE_PANEL);
    expect([link.shownPanel, link.stats]).toEqual([null, null]);
    expect(link.display).toBe(display);
  });

  test('a built Panel reaches the Panel with its parts, and readings feed its gauges', () => {
    const { link, sockets } = connectedLink();
    const socket = latest(sockets);
    socket.receive('{"type":"show_built_panel","title":"Pi","parts":[{"kind":"gauge","source":"cpu"}]}');
    expect([link.shownPanel, link.builtPanel, link.stats]).toEqual([
      'built',
      { title: 'Pi', parts: [{ kind: 'gauge', source: 'cpu' }] },
      null,
    ]);
    socket.receive(STATS);
    expect(link.stats).toEqual(READING);
    socket.receive(SHOW_STATS);
    expect([link.shownPanel, link.builtPanel, link.stats]).toEqual(['stats', null, null]);
  });

  test('a dropped connection hides the Panel and drops the reading', () => {
    const { link, sockets } = connectedLink();
    latest(sockets).receive(SHOW_STATS);
    latest(sockets).receive(STATS);
    latest(sockets).fail();
    expect([link.shownPanel, link.stats]).toEqual([null, null]);
  });

  test('stop hides the Panel', () => {
    const { link, sockets } = connectedLink();
    latest(sockets).receive(SHOW_STATS);
    link.stop();
    expect(link.shownPanel).toBeNull();
  });

  test('each Panel change notifies subscribers once', () => {
    const { link, sockets } = connectedLink();
    const listener = vi.fn();
    link.subscribePanel(listener);
    const socket = latest(sockets);
    socket.receive(SHOW_STATS);
    expect(listener).toHaveBeenCalledTimes(1);
    socket.receive(STATS);
    socket.receive(STATS);
    expect(listener).toHaveBeenCalledTimes(3);
    socket.receive(HIDE_PANEL);
    expect(listener).toHaveBeenCalledTimes(4);
  });

  test('state and level messages, dropped readings and repeats never notify Panel subscribers', () => {
    const { link, sockets } = connectedLink();
    const socket = latest(sockets);
    const listener = vi.fn();
    link.subscribePanel(listener);
    socket.receive(STATS);
    socket.receive(HIDE_PANEL);
    socket.receive(SHOW_STATS);
    listener.mockClear();
    socket.receive(SHOW_STATS);
    for (const state of ['listening', 'speaking']) {
      socket.receive(JSON.stringify({ type: 'state', state }));
      for (let index = 1; index <= 100; index += 1) {
        socket.receive(JSON.stringify({ type: 'level', level: index / 100, source: 'speech' }));
      }
    }
    expect(listener).not.toHaveBeenCalled();
  });

  test('an unsubscribed listener hears nothing more', () => {
    const { link, sockets } = connectedLink();
    const listener = vi.fn();
    const unsubscribe = link.subscribePanel(listener);
    unsubscribe();
    latest(sockets).receive(SHOW_STATS);
    expect(listener).not.toHaveBeenCalled();
  });

  test('closing the Panel sends exactly one PanelClosed and hides it here', () => {
    const { link, sockets } = connectedLink();
    const socket = latest(sockets);
    socket.receive(SHOW_STATS);
    socket.receive(STATS);
    const listener = vi.fn();
    link.subscribePanel(listener);
    expect(link.closePanel()).toBe(true);
    expect(socket.sent).toEqual([PANEL_CLOSED]);
    expect([link.shownPanel, link.stats]).toEqual([null, null]);
    expect(listener).toHaveBeenCalledTimes(1);
    // A second close finds no Panel and sends nothing.
    expect(link.closePanel()).toBe(false);
    expect(socket.sent).toEqual([PANEL_CLOSED]);
  });

  test('closing with no Panel shown sends nothing', () => {
    const { link, sockets } = connectedLink();
    expect(link.closePanel()).toBe(false);
    expect(latest(sockets).sent).toEqual([]);
  });

  test('a reading after a local close is dropped until Jarvis shows the Panel again', () => {
    const { link, sockets } = connectedLink();
    const socket = latest(sockets);
    socket.receive(SHOW_STATS);
    link.closePanel();
    socket.receive(STATS);
    expect([link.shownPanel, link.stats]).toEqual([null, null]);
  });

  test('a Tap while the Panel shows sends one Tap and leaves the Panel open', () => {
    const { link, sockets } = connectedLink();
    latest(sockets).receive(SHOW_STATS);
    expect(link.tap()).toBe(true);
    expect(latest(sockets).sent).toEqual([TAP]);
    expect(link.shownPanel).toBe('stats');
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

describe('timers', () => {
  const TIMERS =
    '{"type":"timers","timers":[{"id":3,"label":"Time to sleep","ends_at_ms":1763830800000,"state":"running"},{"id":4,"ends_at_ms":5,"state":"pending"}]}';

  test('no timers until the first timers message, which keeps its rows and the UI clock at arrival', () => {
    vi.setSystemTime(1_763_829_000_000);
    const { link, sockets } = connectedLink();
    expect(link.shownTimers).toEqual({ timers: [], receivedAtMs: 0 });
    latest(sockets).receive(TIMERS);
    expect(link.shownTimers).toEqual({
      timers: [
        { id: 3, label: 'Time to sleep', endsAtMs: 1763830800000, state: 'running' },
        { id: 4, label: null, endsAtMs: 5, state: 'pending' },
      ],
      receivedAtMs: 1_763_829_000_000,
    });
    latest(sockets).receive('{"type":"timers","timers":[]}');
    expect(link.shownTimers.timers).toEqual([]);
  });

  test('a timers message notifies timer subscribers once and Panel subscribers never; other messages never notify timer subscribers', () => {
    const { link, sockets } = connectedLink();
    let timerCalls = 0;
    let panelCalls = 0;
    link.subscribeTimers(() => (timerCalls += 1));
    link.subscribePanel(() => (panelCalls += 1));
    latest(sockets).receive(TIMERS);
    expect([timerCalls, panelCalls]).toEqual([1, 0]);
    latest(sockets).receive(SHOW_STATS);
    latest(sockets).receive(STATS);
    latest(sockets).receive('{"type":"state","state":"thinking"}');
    latest(sockets).receive('{"type":"timers","timers":[{"id":1,"ends_at_ms":2,"state":"cancelled"}]}');
    expect(timerCalls).toBe(1);
    expect(link.shownTimers.timers).toHaveLength(2);
  });

  test('a dropped connection clears the timers once; Jarvis sends them again on connect', () => {
    const { link, sockets } = connectedLink();
    latest(sockets).receive(TIMERS);
    let calls = 0;
    link.subscribeTimers(() => (calls += 1));
    latest(sockets).fail();
    expect(link.shownTimers.timers).toEqual([]);
    expect(calls).toBe(1);
  });

  test('each answer sends exactly one message with the id, as the Rust side expects', () => {
    const { link, sockets } = connectedLink();
    expect(link.confirmTimer(3)).toBe(true);
    expect(link.rejectTimer(4)).toBe(true);
    expect(link.cancelTimer(5)).toBe(true);
    expect(latest(sockets).sent).toEqual([
      '{"type":"confirm_timer","id":3}',
      '{"type":"reject_timer","id":4}',
      '{"type":"cancel_timer","id":5}',
    ]);
  });

  test('an answer while connecting or offline sends nothing', () => {
    const { create, sockets } = socketFactory();
    const link = new JarvisLink({ createSocket: create });
    link.start();
    const answers = () => [link.confirmTimer(3), link.rejectTimer(3), link.cancelTimer(3)];
    expect(answers()).toEqual([false, false, false]);
    latest(sockets).open();
    latest(sockets).fail();
    expect(answers()).toEqual([false, false, false]);
    expect(latest(sockets).sent).toEqual([]);
    link.stop();
  });
});
