/**
 * What the UI knows about Jarvis: offline, or online with a state and a level.
 * Ported from `Display` in `crates/jarvis-orb/src/link.rs`.
 */

import type { OrbState, ToOrb } from './protocol';

/** Offline carries no state and no level, so they cannot disagree with the connection. */
export type Display =
  | { readonly kind: 'offline' }
  | { readonly kind: 'online'; readonly state: OrbState; readonly level: number };

export const OFFLINE: Display = { kind: 'offline' };

/** The display from the moment of connecting until Jarvis says otherwise. */
export const CONNECTED: Display = { kind: 'online', state: 'idle', level: 0 };

/** Pulls a level into 0 to 1. A level from the wire is already finite. */
function clampUnit(value: number): number {
  return Math.min(1, Math.max(0, value));
}

/**
 * The display after one message. A state change resets the level; a level
 * counts only while listening (mic) or speaking (speech) and is clamped to 0
 * to 1. Offline ignores everything. Returns `display` itself when nothing
 * changes.
 */
export function applyMessage(display: Display, message: ToOrb): Display {
  if (display.kind === 'offline') {
    return display;
  }
  switch (message.type) {
    case 'state':
      return { kind: 'online', state: message.state, level: 0 };
    case 'level': {
      const matches =
        (display.state === 'listening' && message.source === 'mic') ||
        (display.state === 'speaking' && message.source === 'speech');
      if (!matches) {
        return display;
      }
      return { kind: 'online', state: display.state, level: clampUnit(message.level) };
    }
  }
}
