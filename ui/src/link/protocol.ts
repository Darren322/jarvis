/**
 * The wire format of `crates/orb-protocol` (the one message contract), as
 * TypeScript types. Each WebSocket text message is one JSON object.
 *
 * Jarvis to the UI (ToOrb):
 *   {"type":"state","state":"thinking"}
 *   {"type":"level","level":0.25,"source":"mic"}
 *   {"type":"show_panel","panel":"stats"}
 *   {"type":"show_panel","panel":"camera"}
 *   {"type":"show_built_panel","title":"Shopping","parts":[{"kind":"list","title":"To buy","items":["milk","eggs"]},
 *    {"kind":"gauge","source":"cpu"}]}
 *   {"type":"hide_panel"}
 *   {"type":"timers","timers":[{"id":3,"label":"Time to sleep","ends_at_ms":1763830800000,"state":"running"}]}
 *   {"type":"stats","cpu_percent":12.5,"temperature_c":55.0,"ram_used_bytes":2,"ram_total_bytes":8,
 *    "storage":{"available_bytes":207517036544,"total_bytes":251416899584},"llama":"up","bge":"down"}
 * The UI to Jarvis (FromOrb):
 *   {"type":"tap"}
 *   {"type":"panel_closed"}
 *   {"type":"cancel_timer","id":3}   (also "confirm_timer" and "reject_timer")
 *
 * Parsing is as strict as the Rust decoder (`deny_unknown_fields`, `CpuPercent`,
 * the built Panel's limits): anything else parses to `null` and is ignored.
 */

/** What Jarvis is doing. Mirrors `orb_protocol::OrbState`. */
export type OrbState = 'idle' | 'listening' | 'transcribing' | 'thinking' | 'speaking';

/** Whose loudness a level is. Mirrors `orb_protocol::LevelSource`. */
export type LevelSource = 'speech' | 'mic';

/**
 * A Panel the UI can show. Mirrors `orb_protocol::PanelName`. The camera's
 * picture does not come over the WebSocket: the camera Panel loads it from
 * Jarvis's `/camera` route. A built Panel opens with `show_built_panel`, which
 * carries its parts, never by name.
 */
export type PanelName = 'stats' | 'camera' | 'built';

/** A Panel a `show_panel` message may name: every Panel but the built one. */
export type NamedPanel = Exclude<PanelName, 'built'>;

/** A live figure a gauge shows, filled from the latest Stats. Mirrors `orb_protocol::GaugeSource`. */
export type GaugeSource = 'cpu' | 'temperature' | 'ram' | 'storage' | 'llama' | 'bge';

/** One part of a built Panel: the Catalog. Mirrors `orb_protocol::PanelPart`. Text is shown as text, never as markup. */
export type PanelPart =
  | { readonly kind: 'card'; readonly title: string; readonly text: string }
  | { readonly kind: 'list'; readonly title: string; readonly items: readonly string[] }
  | { readonly kind: 'gauge'; readonly source: GaugeSource };

/** A Panel the model built, within the limits below. Mirrors `orb_protocol::BuiltPanel`. */
export interface BuiltPanel {
  readonly title: string;
  readonly parts: readonly PanelPart[];
}

/** The most characters in a built Panel's title. Mirrors `MAX_PANEL_TITLE_CHARS`. */
export const MAX_PANEL_TITLE_CHARS = 40;
/** The fewest and most parts of a built Panel. Mirror `MIN_PANEL_PARTS` and `MAX_PANEL_PARTS`. */
export const MIN_PANEL_PARTS = 1;
export const MAX_PANEL_PARTS = 4;
/** The fewest and most items in a list part. Mirror `MIN_LIST_ITEMS` and `MAX_LIST_ITEMS`. */
export const MIN_LIST_ITEMS = 1;
export const MAX_LIST_ITEMS = 8;
/** The most characters in a card's title or text, a list's title, or one item. Mirrors `MAX_PART_TEXT_CHARS`. */
export const MAX_PART_TEXT_CHARS = 120;

/** Where a timer is. Mirrors `orb_protocol::TimerState`; a cancelled timer is not on the wire. */
export type TimerState = 'pending' | 'running' | 'ended';

/** One timer on the screen. Mirrors `orb_protocol::Timer`. */
export interface Timer {
  /** Jarvis's stored id, a non-negative integer the UI sends back unchanged. */
  readonly id: number;
  /** `null` when the timer has no label (the wire omits the key). */
  readonly label: string | null;
  /** Unix milliseconds, a non-negative integer. */
  readonly endsAtMs: number;
  readonly state: TimerState;
}

/** The most characters in a timer's label. Mirrors `MAX_TIMER_LABEL_CHARS`. */
export const MAX_TIMER_LABEL_CHARS = 60;

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
  | { readonly type: 'show_panel'; readonly panel: NamedPanel }
  | { readonly type: 'show_built_panel'; readonly panel: BuiltPanel }
  | { readonly type: 'hide_panel' }
  | { readonly type: 'stats'; readonly stats: Stats };

/** Every timer to show, as Jarvis sends it on each change; an empty list clears the strip. */
export type TimerMessage = { readonly type: 'timers'; readonly timers: readonly Timer[] };

/** Jarvis to the UI. Mirrors `orb_protocol::ToOrb`. */
export type ToOrb = OrbMessage | PanelMessage | TimerMessage;

/** The UI to Jarvis. Mirrors `orb_protocol::FromOrb`. The UI decides nothing: Jarvis checks each id. */
export type FromOrb =
  | { readonly type: 'tap' }
  | { readonly type: 'panel_closed' }
  | { readonly type: 'cancel_timer'; readonly id: number }
  | { readonly type: 'confirm_timer'; readonly id: number }
  | { readonly type: 'reject_timer'; readonly id: number };

/** The highest CPU percent `orb_protocol::CpuPercent` accepts. */
export const MAX_CPU_PERCENT = 100;

const ORB_STATES: readonly string[] = ['idle', 'listening', 'transcribing', 'thinking', 'speaking'];
const LEVEL_SOURCES: readonly string[] = ['speech', 'mic'];
const NAMED_PANELS: readonly string[] = ['stats', 'camera'];
const GAUGE_SOURCES: readonly string[] = ['cpu', 'temperature', 'ram', 'storage', 'llama', 'bge'];
const TIMER_STATES: readonly string[] = ['pending', 'running', 'ended'];
const SERVICE_HEALTHS: readonly string[] = ['up', 'down'];

const STATS_KEYS = ['type', 'cpu_percent', 'ram_used_bytes', 'ram_total_bytes', 'llama', 'bge'] as const;
/** Optional in Stats: a Pi without a sensor reading omits it. */
const TEMPERATURE_KEY = 'temperature_c';
/** Optional in Stats: omitted when Jarvis could not read the filesystem. */
const STORAGE_KEY = 'storage';
const TIMER_KEYS = ['id', 'ends_at_ms', 'state'] as const;
const STORAGE_KEYS = ['available_bytes', 'total_bytes'] as const;

function isOrbState(value: unknown): value is OrbState {
  return typeof value === 'string' && ORB_STATES.includes(value);
}

function isLevelSource(value: unknown): value is LevelSource {
  return typeof value === 'string' && LEVEL_SOURCES.includes(value);
}

function isNamedPanel(value: unknown): value is NamedPanel {
  return typeof value === 'string' && NAMED_PANELS.includes(value);
}

function isGaugeSource(value: unknown): value is GaugeSource {
  return typeof value === 'string' && GAUGE_SOURCES.includes(value);
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

/**
 * A string of at most `limit` characters, counted as Unicode code points as
 * Rust's `chars()` counts them (a string's `length` counts UTF-16 units: an
 * emoji is 2).
 */
function isText(value: unknown, limit: number): value is string {
  return typeof value === 'string' && [...value].length <= limit;
}

function isTimerState(value: unknown): value is TimerState {
  return typeof value === 'string' && TIMER_STATES.includes(value);
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

/**
 * A `u64` on the Rust side that JavaScript reads exactly: a non-negative safe
 * integer. Ids and times are far below 2^53; Rust accepts larger ones, which
 * the UI rejects rather than misread.
 */
function isWireInteger(value: unknown): value is number {
  return typeof value === 'number' && Number.isSafeInteger(value) && value >= 0;
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

/** One built-Panel part, exactly as `orb_protocol::PanelPart` decodes it, or `null`. */
function parsePart(value: unknown): PanelPart | null {
  if (!isRecord(value)) {
    return null;
  }
  switch (value.kind) {
    case 'card':
      return hasExactKeys(value, ['kind', 'title', 'text']) &&
        isText(value.title, MAX_PART_TEXT_CHARS) &&
        isText(value.text, MAX_PART_TEXT_CHARS)
        ? { kind: 'card', title: value.title, text: value.text }
        : null;
    case 'list': {
      const items = value.items;
      return hasExactKeys(value, ['kind', 'title', 'items']) &&
        isText(value.title, MAX_PART_TEXT_CHARS) &&
        Array.isArray(items) &&
        items.length >= MIN_LIST_ITEMS &&
        items.length <= MAX_LIST_ITEMS &&
        items.every((item) => isText(item, MAX_PART_TEXT_CHARS))
        ? { kind: 'list', title: value.title, items: items as string[] }
        : null;
    }
    case 'gauge':
      return hasExactKeys(value, ['kind', 'source']) && isGaugeSource(value.source)
        ? { kind: 'gauge', source: value.source }
        : null;
    default:
      return null;
  }
}

/** A ShowBuiltPanel: a title and 1 to `MAX_PANEL_PARTS` parts, every one valid; otherwise `null`. */
function parseBuiltPanel(record: Record<string, unknown>): PanelMessage | null {
  const parts = record.parts;
  if (
    !hasExactKeys(record, ['type', 'title', 'parts']) ||
    !isText(record.title, MAX_PANEL_TITLE_CHARS) ||
    !Array.isArray(parts) ||
    parts.length < MIN_PANEL_PARTS ||
    parts.length > MAX_PANEL_PARTS
  ) {
    return null;
  }
  const parsed = parts.map(parsePart);
  if (parsed.some((part) => part === null)) {
    return null;
  }
  return { type: 'show_built_panel', panel: { title: record.title, parts: parsed as PanelPart[] } };
}

/** One timer, exactly as `orb_protocol::Timer` decodes it, or `null`. */
function parseTimer(value: unknown): Timer | null {
  if (!isRecord(value)) {
    return null;
  }
  const hasLabel = Object.hasOwn(value, 'label');
  if (
    !hasExactKeys(value, [...TIMER_KEYS, ...(hasLabel ? ['label'] : [])]) ||
    !isWireInteger(value.id) ||
    !isWireInteger(value.ends_at_ms) ||
    !isTimerState(value.state) ||
    (hasLabel && !isText(value.label, MAX_TIMER_LABEL_CHARS))
  ) {
    return null;
  }
  return {
    id: value.id,
    label: hasLabel ? (value.label as string) : null,
    endsAtMs: value.ends_at_ms,
    state: value.state,
  };
}

/** A `timers` message: an array (possibly empty) of valid timers; otherwise `null`. */
function parseTimers(record: Record<string, unknown>): TimerMessage | null {
  const timers = record.timers;
  if (!hasExactKeys(record, ['type', 'timers']) || !Array.isArray(timers)) {
    return null;
  }
  const parsed = timers.map(parseTimer);
  if (parsed.some((timer) => timer === null)) {
    return null;
  }
  return { type: 'timers', timers: parsed as Timer[] };
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
 * missing or extra field (in a Stats `storage` object or a built Panel's part
 * too), a level that is not a finite number, a Stats reading out of range, or
 * a built Panel with a part not in the Catalog or over a limit. A finite level outside 0 to 1 is kept as sent; the
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
      if (hasExactKeys(record, ['type', 'panel']) && isNamedPanel(record.panel)) {
        return { type: 'show_panel', panel: record.panel };
      }
      return null;
    case 'show_built_panel':
      return parseBuiltPanel(record);
    case 'hide_panel':
      return hasExactKeys(record, ['type']) ? { type: 'hide_panel' } : null;
    case 'timers':
      return parseTimers(record);
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
