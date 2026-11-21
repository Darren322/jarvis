/**
 * The wire format of `crates/orb-protocol` (the one message contract), as
 * TypeScript types. Each WebSocket text message is one JSON object.
 *
 * Jarvis to the UI (ToOrb):
 *   {"type":"state","state":"thinking"}
 *   {"type":"level","level":0.25,"source":"mic"}
 *   {"type":"show_panel","panel":"stats"}
 *   {"type":"show_panel","panel":"camera"}
 *   {"type":"hide_panel"}
 *   {"type":"stats","cpu_percent":12.5,"temperature_c":55.0,"ram_used_bytes":2,"ram_total_bytes":8,
 *    "storage":{"available_bytes":207517036544,"total_bytes":251416899584},"llama":"up","bge":"down"}
 * The UI to Jarvis (FromOrb):
 *   {"type":"tap"}
 *   {"type":"panel_closed"}
 *
 * Parsing is as strict as the Rust decoder (`deny_unknown_fields`, `CpuPercent`):
 * anything else parses to `null` and is ignored.
 */

/** What Jarvis is doing. Mirrors `orb_protocol::OrbState`. */
export type OrbState = 'idle' | 'listening' | 'transcribing' | 'thinking' | 'speaking';

/** Whose loudness a level is. Mirrors `orb_protocol::LevelSource`. */
export type LevelSource = 'speech' | 'mic';

/**
 * A Panel the UI can show. Mirrors `orb_protocol::PanelName`. The camera's
 * picture does not come over the WebSocket: the camera Panel loads it from
 * Jarvis's `/camera` route.
 */
export type PanelName = 'stats' | 'camera';

/** Whether a service answered its last health check. Mirrors `orb_protocol::ServiceHealth`. */
export type ServiceHealth = 'up' | 'down';

/**
 * The space on the 8 GB Pi's root filesystem, as the `storage` object of a
 * Stats message carries it. Both are non-negative integers; the Dashboard
 * works out what is used (`total - available`).
 */
export interface StorageSpace {
  readonly availableBytes: number;
  readonly totalBytes: number;
}

/** One reading of the Pi's health, as `ToOrb::Stats` carries it. */
export interface Stats {
  /** 0 to 100 inclusive. */
  readonly cpuPercent: number;
  /** `null` when the Pi has no sensor reading (the wire omits the key); never 0 for "unknown". */
  readonly temperatureC: number | null;
  /** A non-negative integer. */
  readonly ramUsedBytes: number;
  /** A non-negative integer. */
  readonly ramTotalBytes: number;
  /** `null` when Jarvis could not read the filesystem (the wire omits the key); never 0 for "unknown". */
  readonly storage: StorageSpace | null;
  readonly llama: ServiceHealth;
  readonly bge: ServiceHealth;
}

/** The messages that drive the Orb: read by its frame loop, never by React. */
export type OrbMessage =
  | { readonly type: 'state'; readonly state: OrbState }
  | { readonly type: 'level'; readonly level: number; readonly source: LevelSource };

/** The messages that open, fill and close a Panel. */
export type PanelMessage =
  | { readonly type: 'show_panel'; readonly panel: PanelName }
  | { readonly type: 'hide_panel' }
  | { readonly type: 'stats'; readonly stats: Stats };

/** Jarvis to the UI. Mirrors `orb_protocol::ToOrb`. */
export type ToOrb = OrbMessage | PanelMessage;

/** The UI to Jarvis. Mirrors `orb_protocol::FromOrb`. */
export type FromOrb = { readonly type: 'tap' } | { readonly type: 'panel_closed' };

/** The highest CPU percent `orb_protocol::CpuPercent` accepts. */
export const MAX_CPU_PERCENT = 100;

const ORB_STATES: readonly string[] = ['idle', 'listening', 'transcribing', 'thinking', 'speaking'];
const LEVEL_SOURCES: readonly string[] = ['speech', 'mic'];
const PANEL_NAMES: readonly string[] = ['stats', 'camera'];
const SERVICE_HEALTHS: readonly string[] = ['up', 'down'];

const STATS_KEYS = ['type', 'cpu_percent', 'ram_used_bytes', 'ram_total_bytes', 'llama', 'bge'] as const;
/** Optional in Stats: a Pi without a sensor reading omits it. */
const TEMPERATURE_KEY = 'temperature_c';
/** Optional in Stats: omitted when Jarvis could not read the filesystem. */
const STORAGE_KEY = 'storage';
const STORAGE_KEYS = ['available_bytes', 'total_bytes'] as const;

function isOrbState(value: unknown): value is OrbState {
  return typeof value === 'string' && ORB_STATES.includes(value);
}

function isLevelSource(value: unknown): value is LevelSource {
  return typeof value === 'string' && LEVEL_SOURCES.includes(value);
}

function isPanelName(value: unknown): value is PanelName {
  return typeof value === 'string' && PANEL_NAMES.includes(value);
}

function isServiceHealth(value: unknown): value is ServiceHealth {
  return typeof value === 'string' && SERVICE_HEALTHS.includes(value);
}

function isFiniteNumber(value: unknown): value is number {
  return typeof value === 'number' && Number.isFinite(value);
}

/** A `u64` on the Rust side. */
function isByteCount(value: unknown): value is number {
  return typeof value === 'number' && Number.isInteger(value) && value >= 0;
}

/** A finite number from 0 to `MAX_CPU_PERCENT` inclusive, as `CpuPercent::new` accepts. */
function isCpuPercent(value: unknown): value is number {
  return isFiniteNumber(value) && value >= 0 && value <= MAX_CPU_PERCENT;
}

/** True when `record` has exactly these keys, as the Rust side's `deny_unknown_fields` requires. */
function hasExactKeys(record: object, keys: readonly string[]): boolean {
  const own = Object.keys(record);
  return own.length === keys.length && keys.every((key) => own.includes(key));
}

/**
 * A Stats `storage` object: exactly `available_bytes` and `total_bytes`, both
 * byte counts. Anything else, `null` included, gives `undefined`.
 */
function parseStorage(value: unknown): StorageSpace | undefined {
  if (typeof value !== 'object' || value === null || Array.isArray(value) || !hasExactKeys(value, STORAGE_KEYS)) {
    return undefined;
  }
  const record = value as Record<string, unknown>;
  if (!isByteCount(record.available_bytes) || !isByteCount(record.total_bytes)) {
    return undefined;
  }
  return { availableBytes: record.available_bytes, totalBytes: record.total_bytes };
}

function parseStats(record: Record<string, unknown>): PanelMessage | null {
  const hasTemperature = Object.hasOwn(record, TEMPERATURE_KEY);
  const hasStorage = Object.hasOwn(record, STORAGE_KEY);
  const keys = [
    ...STATS_KEYS,
    ...(hasTemperature ? [TEMPERATURE_KEY] : []),
    ...(hasStorage ? [STORAGE_KEY] : []),
  ];
  const temperature = record[TEMPERATURE_KEY];
  // An absent key is no reading (`null`); a present but malformed one is `undefined`.
  const storage = hasStorage ? parseStorage(record[STORAGE_KEY]) : null;
  if (
    !hasExactKeys(record, keys) ||
    !isCpuPercent(record.cpu_percent) ||
    (hasTemperature && !isFiniteNumber(temperature)) ||
    storage === undefined ||
    !isByteCount(record.ram_used_bytes) ||
    !isByteCount(record.ram_total_bytes) ||
    !isServiceHealth(record.llama) ||
    !isServiceHealth(record.bge)
  ) {
    return null;
  }
  return {
    type: 'stats',
    stats: {
      cpuPercent: record.cpu_percent,
      temperatureC: isFiniteNumber(temperature) ? temperature : null,
      ramUsedBytes: record.ram_used_bytes,
      ramTotalBytes: record.ram_total_bytes,
      storage,
      llama: record.llama,
      bge: record.bge,
    },
  };
}

/**
 * Reads one message from Jarvis. Returns `null` for anything that is not a
 * well-formed ToOrb: not JSON, not an object, an unknown type or Panel name, a
 * missing or extra field (in a Stats `storage` object too), a level that is
 * not a finite number, or a Stats reading out of range. A finite level outside 0 to 1 is kept as sent; the
 * display clamps it.
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
        isFiniteNumber(record.level) &&
        isLevelSource(record.source)
      ) {
        return { type: 'level', level: record.level, source: record.source };
      }
      return null;
    case 'show_panel':
      if (hasExactKeys(record, ['type', 'panel']) && isPanelName(record.panel)) {
        return { type: 'show_panel', panel: record.panel };
      }
      return null;
    case 'hide_panel':
      return hasExactKeys(record, ['type']) ? { type: 'hide_panel' } : null;
    case 'stats':
      return parseStats(record);
    default:
      return null;
  }
}

/** One message to Jarvis, as the text of one WebSocket message. */
export function encodeFromOrb(message: FromOrb): string {
  return JSON.stringify(message);
}
