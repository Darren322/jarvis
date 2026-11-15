import type { CreateSocket, SocketLike } from '../../src/link/link';

/** A WebSocket stand-in the test drives by hand. No network. */
export class FakeSocket implements SocketLike {
  onopen: ((event: Event) => void) | null = null;
  onmessage: ((event: MessageEvent) => void) | null = null;
  onclose: ((event: CloseEvent) => void) | null = null;
  onerror: ((event: Event) => void) | null = null;
  readonly sent: string[] = [];
  closeCalls = 0;

  constructor(readonly url: string) {}

  send(data: string): void {
    this.sent.push(data);
  }

  close(): void {
    this.closeCalls += 1;
  }

  /** Jarvis accepts the connection. */
  open(): void {
    this.onopen?.(new Event('open'));
  }

  /** Jarvis sends one text (or other) message. */
  receive(data: unknown): void {
    this.onmessage?.(new MessageEvent('message', { data }));
  }

  /** The connection fails or Jarvis goes away: `error`, then `close`, as browsers do. */
  fail(): void {
    this.onerror?.(new Event('error'));
    this.onclose?.(new CloseEvent('close'));
  }
}

/** A `CreateSocket` that records every socket it makes. */
export function socketFactory(): { create: CreateSocket; sockets: FakeSocket[] } {
  const sockets: FakeSocket[] = [];
  return {
    sockets,
    create: (url) => {
      const socket = new FakeSocket(url);
      sockets.push(socket);
      return socket;
    },
  };
}

/** The newest socket, failing the test if there is none. */
export function latest(sockets: FakeSocket[]): FakeSocket {
  const socket = sockets.at(-1);
  if (socket === undefined) {
    throw new Error('no socket was opened');
  }
  return socket;
}
