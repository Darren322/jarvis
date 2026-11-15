/**
 * The wire format of `crates/orb-protocol` (the one message contract), as
 * TypeScript types. Each WebSocket text message is one JSON object.
 *
 * Jarvis to the UI (ToOrb):
 *   {"type":"state","state":"thinking"}
 *   {"type":"level","level":0.25,"source":"mic"}
 * The UI to Jarvis (FromOrb):
 *   {"type":"tap"}
 *
 * Other message types (such as the panel messages of jarvis-ui ticket 07) parse
 * to `null` here, so the Orb ignores them.
 */

/** What Jarvis is doing. Mirrors `orb_protocol::OrbState`. */
export type OrbState = 'idle' | 'listening' | 'transcribing' | 'thinking' | 'speaking';

/** Whose loudness a level is. Mirrors `orb_protocol::LevelSource`. */
export type LevelSource = 'speech' | 'mic';

/** Jarvis to the UI. Mirrors `orb_protocol::ToOrb` (the variants the Orb uses). */
export type ToOrb =
  | { readonly type: 'state'; readonly state: OrbState }
  | { readonly type: 'level'; readonly level: number; readonly source: LevelSource };

/** The UI to Jarvis. Mirrors `orb_protocol::FromOrb`. */
export type FromOrb = { readonly type: 'tap' };

const ORB_STATES: readonly string[] = ['idle', 'listening', 'transcribing', 'thinking', 'speaking'];
const LEVEL_SOURCES: readonly string[] = ['speech', 'mic'];

function isOrbState(value: unknown): value is OrbState {
  return typeof value === 'string' && ORB_STATES.includes(value);
}

function isLevelSource(value: unknown): value is LevelSource {
  return typeof value === 'string' && LEVEL_SOURCES.includes(value);
}

/** True when `record` has exactly these keys, as the Rust side's `deny_unknown_fields` requires. */
function hasExactKeys(record: object, keys: readonly string[]): boolean {
  const own = Object.keys(record);
  return own.length === keys.length && keys.every((key) => own.includes(key));
}

/**
 * Reads one message from Jarvis. Returns `null` for anything that is not a
 * well-formed ToOrb the Orb understands: not JSON, not an object, an unknown
 * type, a missing or extra field, or a level that is not a finite number. A
 * finite level outside 0 to 1 is kept as sent; the reader clamps it.
 */
export function parseToOrb(text: string): ToOrb | null {
  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch {
    return null;
  }
  if (typeof value !== 'object' || value === null || Array.isArray(value)) {
    return null;
  }
  const record = value as Record<string, unknown>;
  switch (record.type) {
    case 'state':
      if (hasExactKeys(record, ['type', 'state']) && isOrbState(record.state)) {
        return { type: 'state', state: record.state };
      }
      return null;
    case 'level':
      if (
        hasExactKeys(record, ['type', 'level', 'source']) &&
        typeof record.level === 'number' &&
        Number.isFinite(record.level) &&
        isLevelSource(record.source)
      ) {
        return { type: 'level', level: record.level, source: record.source };
      }
      return null;
    default:
      return null;
  }
}

/** One message to Jarvis, as the text of one WebSocket message. */
export function encodeFromOrb(message: FromOrb): string {
  return JSON.stringify(message);
}
