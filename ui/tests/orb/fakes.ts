import type { FrameScheduler } from '../../src/orb/Orb';
import type { CreateOrbRenderer, OrbFrame, OrbRenderer } from '../../src/orb/renderer';

/** What a fake renderer saw in one frame, copied (the loop reuses its frame object). */
export interface SeenFrame {
  readonly view: OrbFrame['view'];
  readonly level: number;
  readonly lookJson: string;
  readonly width: number;
  readonly height: number;
  readonly clock: number;
}

/** A renderer that draws nothing and records every frame. jsdom has no WebGL. */
export class FakeRenderer implements OrbRenderer {
  readonly frames: SeenFrame[] = [];
  disposed = false;

  constructor(readonly host: HTMLElement | null = null) {}

  draw(frame: OrbFrame): void {
    this.frames.push({
      view: frame.view,
      level: frame.level,
      lookJson: JSON.stringify(frame.look),
      width: frame.width,
      height: frame.height,
      clock: frame.clock,
    });
  }

  dispose(): void {
    this.disposed = true;
  }

  get last(): SeenFrame {
    const frame = this.frames.at(-1);
    if (frame === undefined) {
      throw new Error('nothing was drawn');
    }
    return frame;
  }
}

/** A `CreateOrbRenderer` that records the renderers it makes. */
export function fakeRenderers(): { create: CreateOrbRenderer; made: FakeRenderer[] } {
  const made: FakeRenderer[] = [];
  return {
    made,
    create: (host) => {
      const renderer = new FakeRenderer(host);
      made.push(renderer);
      return renderer;
    },
  };
}

/** A frame scheduler the test steps by hand. */
export class ManualFrames implements FrameScheduler {
  #pending = new Map<number, (timeMs: number) => void>();
  #nextId = 1;

  request(callback: (timeMs: number) => void): number {
    const id = this.#nextId;
    this.#nextId += 1;
    this.#pending.set(id, callback);
    return id;
  }

  cancel(id: number): void {
    this.#pending.delete(id);
  }

  get pending(): number {
    return this.#pending.size;
  }

  /** Runs every callback waiting for a frame, at `timeMs`. */
  tick(timeMs: number): void {
    const due = [...this.#pending.values()];
    this.#pending.clear();
    for (const callback of due) {
      callback(timeMs);
    }
  }
}
