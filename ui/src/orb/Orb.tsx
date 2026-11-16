import { type KeyboardEvent, useEffect, useRef } from 'react';
import type { OrbSource } from '../link/link';
import { OrbAnimator } from './animator';
import type { CreateOrbRenderer } from './renderer';

/** The most device pixels per CSS pixel the Orb draws at; more costs fill rate for no visible gain. */
export const MAX_PIXEL_RATIO = 2;

/** Calls a function on the next display frame. Production uses `requestAnimationFrame`; tests pass their own. */
export interface FrameScheduler {
  /** Schedules `callback` with a timestamp in milliseconds; returns an id for `cancel`. */
  request(callback: (timeMs: number) => void): number;
  cancel(id: number): void;
}

const animationFrames: FrameScheduler = {
  request: (callback) => requestAnimationFrame(callback),
  cancel: (id) => cancelAnimationFrame(id),
};

export interface OrbSize {
  readonly width: number;
  readonly height: number;
}

export interface OrbPosition {
  readonly left: number;
  readonly top: number;
}

export interface OrbProps {
  /** Where the state and level come from; read every frame, never through React state. */
  readonly source: OrbSource;
  /** Called on a tap. The caller decides whether anything is sent (offline sends nothing). */
  readonly onTap: () => void;
  /** Drawing area in CSS pixels. The look does not depend on it. */
  readonly size: OrbSize;
  /** Top-left corner in CSS pixels, relative to the nearest positioned ancestor. */
  readonly position: OrbPosition;
  /** Clips the drawing to the circle inside the area, for a small Orb whose outer rings would otherwise be cut straight by its square. Default: the whole area. */
  readonly round?: boolean;
  readonly createRenderer: CreateOrbRenderer;
  /** Default: `requestAnimationFrame`. */
  readonly frames?: FrameScheduler;
}

/**
 * The animated figure that shows Jarvis's state. It renders once per prop
 * change; every animation frame runs outside React, in its own loop.
 */
export function Orb({ source, onTap, size, position, round = false, createRenderer, frames = animationFrames }: OrbProps) {
  const hostRef = useRef<HTMLDivElement>(null);
  const sizeRef = useRef(size);

  useEffect(() => {
    sizeRef.current = size;
  }, [size]);

  useEffect(() => {
    const host = hostRef.current;
    if (host === null) {
      return;
    }
    const renderer = createRenderer(host);
    if (renderer === null) {
      return;
    }
    const animator = new OrbAnimator(source, renderer);
    let id = 0;
    const loop = (timeMs: number) => {
      const { width, height } = sizeRef.current;
      const pixelRatio = Math.min(MAX_PIXEL_RATIO, window.devicePixelRatio || 1);
      animator.frame(timeMs / 1000, width, height, pixelRatio);
      id = frames.request(loop);
    };
    id = frames.request(loop);
    return () => {
      frames.cancel(id);
      renderer.dispose();
    };
  }, [source, createRenderer, frames]);

  const onKeyDown = (event: KeyboardEvent) => {
    if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault();
      onTap();
    }
  };

  return (
    <div
      ref={hostRef}
      role="button"
      tabIndex={0}
      aria-label="Jarvis orb: tap to talk"
      onPointerDown={onTap}
      onKeyDown={onKeyDown}
      style={{
        position: 'absolute',
        left: position.left,
        top: position.top,
        width: size.width,
        height: size.height,
        overflow: 'hidden',
        borderRadius: round ? '50%' : undefined,
        touchAction: 'none',
        outline: 'none',
      }}
    />
  );
}
