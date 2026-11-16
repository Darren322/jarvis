/**
 * The UI's connection to Jarvis: one WebSocket on localhost.
 *
 * Jarvis is the server. The link connects, reads ToOrb messages into a small
 * store and sends a Tap or a PanelClosed when asked. It holds no logic about
 * what a tap means (jarvis-ui D2). The store has two parts:
 * - the display (state and level), read by the Orb's frame loop directly, so
 *   the loudness (about 30 messages a second) never goes through React state;
 * - the Panel (which one shows, and its latest reading), which React reads with
 *   `useSyncExternalStore`. Only Panel messages notify its subscribers.
 *
 * Nothing polls: the only timer is the reconnect timer, started when a
 * connection closes or fails.
 */

import { applyMessage, CONNECTED, type Display, OFFLINE } from './display';
import { applyPanelMessage, NO_PANEL, type PanelView } from './panel';
import { encodeFromOrb, type FromOrb, type PanelName, parseToOrb, type Stats } from './protocol';

/**
 * The port of Jarvis's WebSocket. It must match `DEFAULT_PORT` (8765, the
 * default of `JARVIS_ORB_PORT`) in `src/config/orb.rs`.
 */
export const JARVIS_PORT = 8765;

/** The address of Jarvis's WebSocket: localhost only, route `/`. */
export const JARVIS_URL = `ws://127.0.0.1:${JARVIS_PORT}/`;

/**
 * How long the link waits after a connection closes or fails before it tries
 * again, in milliseconds. The same 3 s as `RECONNECT_INTERVAL` in
 * `crates/jarvis-orb/src/link.rs` (visual-jarvis D3: "every few seconds").
 */
export const RECONNECT_DELAY = 3000;

/** Where the link is with its connection. */
export type LinkStatus =
  /** A socket is open and Jarvis is reachable. */
  | 'connected'
  /** A socket is being opened. */
  | 'connecting'
  /** No socket: waiting `RECONNECT_DELAY` to try again, or stopped. */
  | 'offline';

/** The parts of a browser `WebSocket` the link uses, so tests can pass a fake. */
export interface SocketLike {
  onopen: ((event: Event) => void) | null;
  onmessage: ((event: MessageEvent) => void) | null;
  onclose: ((event: CloseEvent) => void) | null;
  onerror: ((event: Event) => void) | null;
  send(data: string): void;
  close(): void;
}

export type CreateSocket = (url: string) => SocketLike;

export interface LinkOptions {
  /** Default `JARVIS_URL`. */
  readonly url?: string;
  /** Default: the browser's `WebSocket`. */
  readonly createSocket?: CreateSocket;
}

/** What the Orb reads every frame. Reading never allocates or re-renders. */
export interface OrbSource {
  /** What the link last learned from Jarvis; offline while not connected. */
  readonly display: Display;
}

/** What the Panel host reads, through React. */
export interface PanelSource {
  /** Calls `listener` after every change to `shownPanel` or `stats`; returns the unsubscribe. */
  readonly subscribePanel: (listener: () => void) => () => void;
  /** The Panel Jarvis asked for, or `null`. */
  readonly shownPanel: PanelName | null;
  /** The Dashboard's latest reading; `null` before the first one and while no Panel shows. */
  readonly stats: Stats | null;
  /** Darren tapped outside the Panel: tell Jarvis once and hide it here. */
  readonly closePanel: () => void;
}

const browserSocket: CreateSocket = (url) => new WebSocket(url);

export class JarvisLink implements OrbSource, PanelSource {
  #display: Display = OFFLINE;
  #panel: PanelView = NO_PANEL;
  readonly #panelListeners = new Set<() => void>();
  #status: LinkStatus = 'offline';
  #socket: SocketLike | null = null;
  #retry: ReturnType<typeof setTimeout> | null = null;
  #running = false;
  readonly #url: string;
  readonly #createSocket: CreateSocket;

  constructor(options: LinkOptions = {}) {
    this.#url = options.url ?? JARVIS_URL;
    this.#createSocket = options.createSocket ?? browserSocket;
  }

  get display(): Display {
    return this.#display;
  }

  get status(): LinkStatus {
    return this.#status;
  }

  get shownPanel(): PanelName | null {
    return this.#panel.kind === 'none' ? null : this.#panel.kind;
  }

  get stats(): Stats | null {
    return this.#panel.kind === 'stats' ? this.#panel.reading : null;
  }

  /** An arrow function, so `useSyncExternalStore` can be given it as it is. */
  readonly subscribePanel = (listener: () => void): (() => void) => {
    this.#panelListeners.add(listener);
    return () => {
      this.#panelListeners.delete(listener);
    };
  };

  /** Starts connecting. Does nothing if already started. */
  start(): void {
    if (this.#running) {
      return;
    }
    this.#running = true;
    this.#connect();
  }

  /** Closes the socket and cancels any reconnect. The link may be started again. */
  stop(): void {
    this.#running = false;
    if (this.#retry !== null) {
      clearTimeout(this.#retry);
      this.#retry = null;
    }
    this.#drop();
  }

  /**
   * Sends one Tap if connected and returns true; returns false and sends
   * nothing while connecting or offline. An arrow function, so it can be passed
   * as an event handler.
   */
  readonly tap = (): boolean => this.#send({ type: 'tap' });

  /**
   * Hides the shown Panel and sends one PanelClosed; Jarvis answers nothing, so
   * hiding it here is what closes it on screen. Returns false and does nothing
   * when no Panel shows. A Panel shows only while connected (a drop hides it).
   */
  readonly closePanel = (): boolean => {
    if (this.#panel.kind === 'none') {
      return false;
    }
    this.#setPanel(NO_PANEL);
    return this.#send({ type: 'panel_closed' });
  };

  #send(message: FromOrb): boolean {
    const socket = this.#socket;
    if (this.#status !== 'connected' || socket === null) {
      return false;
    }
    socket.send(encodeFromOrb(message));
    return true;
  }

  #setPanel(next: PanelView): void {
    if (next === this.#panel) {
      return;
    }
    this.#panel = next;
    for (const listener of [...this.#panelListeners]) {
      listener();
    }
  }

  #connect(): void {
    this.#retry = null;
    this.#status = 'connecting';
    let socket: SocketLike;
    try {
      socket = this.#createSocket(this.#url);
    } catch {
      // The browser throws only for a bad URL; keep trying like any other failure.
      this.#status = 'offline';
      this.#scheduleRetry();
      return;
    }
    this.#socket = socket;
    socket.onopen = () => {
      this.#status = 'connected';
      this.#display = CONNECTED;
    };
    socket.onmessage = (event) => {
      if (typeof event.data !== 'string') {
        return;
      }
      const message = parseToOrb(event.data);
      switch (message?.type) {
        case undefined:
          return;
        case 'state':
        case 'level':
          this.#display = applyMessage(this.#display, message);
          return;
        case 'show_panel':
        case 'hide_panel':
        case 'stats':
          this.#setPanel(applyPanelMessage(this.#panel, message));
          return;
      }
    };
    // A failed connection fires `error` and then `close`, so only `close` acts.
    socket.onclose = () => {
      this.#drop();
      this.#scheduleRetry();
    };
  }

  /** Forgets the socket (closing it if open), shows offline and hides the Panel. */
  #drop(): void {
    const socket = this.#socket;
    this.#socket = null;
    this.#status = 'offline';
    this.#display = OFFLINE;
    this.#setPanel(NO_PANEL);
    if (socket !== null) {
      socket.onopen = null;
      socket.onmessage = null;
      socket.onclose = null;
      socket.onerror = null;
      socket.close();
    }
  }

  #scheduleRetry(): void {
    if (!this.#running || this.#retry !== null) {
      return;
    }
    this.#retry = setTimeout(() => this.#connect(), RECONNECT_DELAY);
  }
}
