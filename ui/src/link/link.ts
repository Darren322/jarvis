/**
 * The UI's connection to Jarvis: one WebSocket on localhost.
 *
 * Jarvis is the server. The link connects, reads ToOrb messages into a small
 * store and sends a Tap when asked. It holds no logic about what a tap means
 * (jarvis-ui D2). The store is read by the Orb's frame loop directly, so the
 * loudness (about 30 messages a second) never goes through React state.
 *
 * Nothing polls: the only timer is the reconnect timer, started when a
 * connection closes or fails.
 */

import { applyMessage, CONNECTED, type Display, OFFLINE } from './display';
import { encodeFromOrb, parseToOrb } from './protocol';

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

const browserSocket: CreateSocket = (url) => new WebSocket(url);

export class JarvisLink implements OrbSource {
  #display: Display = OFFLINE;
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
  readonly tap = (): boolean => {
    const socket = this.#socket;
    if (this.#status !== 'connected' || socket === null) {
      return false;
    }
    socket.send(encodeFromOrb({ type: 'tap' }));
    return true;
  };

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
      if (message !== null) {
        this.#display = applyMessage(this.#display, message);
      }
    };
    // A failed connection fires `error` and then `close`, so only `close` acts.
    socket.onclose = () => {
      this.#drop();
      this.#scheduleRetry();
    };
  }

  /** Forgets the socket (closing it if open) and shows offline. */
  #drop(): void {
    const socket = this.#socket;
    this.#socket = null;
    this.#status = 'offline';
    this.#display = OFFLINE;
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
