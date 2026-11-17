import { describe, expect, test } from 'vitest';
import { encodeFromOrb, parseToOrb } from '../../src/link/protocol';

// Wire examples from the doc comments in crates/orb-protocol/src/lib.rs.
test.each([
  ['{"type":"state","state":"idle"}', { type: 'state', state: 'idle' }],
  ['{"type":"state","state":"listening"}', { type: 'state', state: 'listening' }],
  ['{"type":"state","state":"transcribing"}', { type: 'state', state: 'transcribing' }],
  ['{"type":"state","state":"thinking"}', { type: 'state', state: 'thinking' }],
  ['{"type":"state","state":"speaking"}', { type: 'state', state: 'speaking' }],
  ['{"type":"level","level":0.25,"source":"mic"}', { type: 'level', level: 0.25, source: 'mic' }],
  ['{"type":"level","level":1,"source":"speech"}', { type: 'level', level: 1, source: 'speech' }],
  // Out of range is kept as sent; the display clamps it.
  ['{"type":"level","level":1.01,"source":"mic"}', { type: 'level', level: 1.01, source: 'mic' }],
  [' {"source":"mic","level":0,"type":"level"} ', { type: 'level', level: 0, source: 'mic' }],
])('%s parses', (text, message) => {
  expect(parseToOrb(text)).toEqual(message);
});

test.each([
  'not json',
  '{"type":"State","state":"idle"}',
  '{"type":"state","state":"Idle"}',
  '{"type":"level","level":"0.5","source":"mic"}',
  '{"type":"level","level":0.5,"source":"mic","extra":true}',
  '{"type":"tap"}',
])('%s is ignored', (text) => {
  expect(parseToOrb(text)).toBeNull();
});

test.each([
  [{ type: 'tap' }, '{"type":"tap"}'],
  [{ type: 'panel_closed' }, '{"type":"panel_closed"}'],
] as const)('%o is encoded as the Rust side expects', (message, text) => {
  expect(encodeFromOrb(message)).toBe(text);
});

describe('panel messages', () => {
  test.each([
    ['{"type":"show_panel","panel":"stats"}', { type: 'show_panel', panel: 'stats' }],
    ['{"type":"hide_panel"}', { type: 'hide_panel' }],
  ])('%s parses', (text, message) => {
    expect(parseToOrb(text)).toEqual(message);
  });

  test.each([
    '{"type":"show_panel","panel":"camera"}',
    '{"type":"show_panel","panel":"Stats"}',
    '{"type":"show_panel"}',
    '{"type":"show_panel","panel":"stats","extra":1}',
    '{"type":"hide_panel","panel":"stats"}',
  ])('%s is ignored', (text) => {
    expect(parseToOrb(text)).toBeNull();
  });
});

describe('stats', () => {
  /** The wire example from crates/orb-protocol, with fields replaced or removed. */
  function stats(changes: Record<string, unknown> = {}, without: readonly string[] = []): string {
    const record: Record<string, unknown> = {
      type: 'stats',
      cpu_percent: 12.5,
      temperature_c: 55.0,
      ram_used_bytes: 2,
      ram_total_bytes: 8,
      storage: { available_bytes: 207_517_036_544, total_bytes: 251_416_899_584 },
      llama: 'up',
      bge: 'down',
      ...changes,
    };
    for (const key of without) {
      delete record[key];
    }
    return JSON.stringify(record);
  }

  test('the wire example parses into a reading', () => {
    expect(
      parseToOrb(
        '{"type":"stats","cpu_percent":12.5,"temperature_c":55.0,"ram_used_bytes":2,"ram_total_bytes":8,"storage":{"available_bytes":207517036544,"total_bytes":251416899584},"llama":"up","bge":"down"}',
      ),
    ).toEqual({
      type: 'stats',
      stats: {
        cpuPercent: 12.5,
        temperatureC: 55,
        ramUsedBytes: 2,
        ramTotalBytes: 8,
        storage: { availableBytes: 207_517_036_544, totalBytes: 251_416_899_584 },
        llama: 'up',
        bge: 'down',
      },
    });
  });

  test('an absent storage parses as no reading, not 0', () => {
    expect(parseToOrb(stats({}, ['storage']))).toMatchObject({ stats: { storage: null } });
  });

  test('a reading without both temperature and storage parses', () => {
    expect(parseToOrb(stats({}, ['temperature_c', 'storage']))).toMatchObject({
      stats: { temperatureC: null, storage: null },
    });
  });

  /** The wire example with its `storage` object's fields replaced or removed. */
  function storage(changes: Record<string, unknown> = {}, without: readonly string[] = []): string {
    const object: Record<string, unknown> = { available_bytes: 207_517_036_544, total_bytes: 251_416_899_584, ...changes };
    for (const key of without) {
      delete object[key];
    }
    return stats({ storage: object });
  }

  test.each([
    // [field, value, accepted]
    ['available_bytes', -1, false],
    ['available_bytes', 0, true],
    ['available_bytes', 1.5, false],
    ['available_bytes', '2', false],
    ['available_bytes', null, false],
    ['total_bytes', -1, false],
    ['total_bytes', 0, true],
    ['total_bytes', 251_416_899_584, true],
    ['total_bytes', 8.5, false],
    ['total_bytes', '8', false],
    ['total_bytes', null, false],
  ])('storage %s of %o is accepted: %s', (field, value, accepted) => {
    expect(parseToOrb(storage({ [field]: value })) !== null).toBe(accepted);
  });

  test.each(['available_bytes', 'total_bytes'])('a storage object without %s is ignored', (field) => {
    expect(parseToOrb(storage({}, [field]))).toBeNull();
  });

  test('a storage object with an extra field is ignored', () => {
    expect(parseToOrb(storage({ used_bytes: 1 }))).toBeNull();
  });

  test.each([null, 0, '251416899584', {}, [], [207_517_036_544, 251_416_899_584]])(
    'a storage of %o is ignored',
    (value) => {
      expect(parseToOrb(stats({ storage: value }))).toBeNull();
    },
  );

  test('an absent temperature parses as no reading, not 0', () => {
    expect(parseToOrb(stats({}, ['temperature_c']))).toMatchObject({ stats: { temperatureC: null } });
  });

  test.each([
    // [field, value, accepted]
    ['cpu_percent', -0.01, false],
    ['cpu_percent', 0, true],
    ['cpu_percent', 100, true],
    ['cpu_percent', 100.01, false],
    ['cpu_percent', null, false],
    ['cpu_percent', '12', false],
    ['temperature_c', -5, true],
    ['temperature_c', 0, true],
    ['temperature_c', null, false],
    ['temperature_c', '55', false],
    ['ram_used_bytes', -1, false],
    ['ram_used_bytes', 0, true],
    ['ram_used_bytes', 1.5, false],
    ['ram_used_bytes', '2', false],
    ['ram_total_bytes', -1, false],
    ['ram_total_bytes', 0, true],
    ['ram_total_bytes', 8_464_105_472, true],
    ['ram_total_bytes', 8.5, false],
    ['llama', 'up', true],
    ['llama', 'down', true],
    ['llama', 'Up', false],
    ['llama', true, false],
    ['bge', 'unknown', false],
  ])('%s of %o is accepted: %s', (field, value, accepted) => {
    expect(parseToOrb(stats({ [field]: value })) !== null).toBe(accepted);
  });

  test.each(['cpu_percent', 'ram_used_bytes', 'ram_total_bytes', 'llama', 'bge'])(
    'a reading without %s is ignored',
    (field) => {
      expect(parseToOrb(stats({}, [field]))).toBeNull();
    },
  );

  test('a reading with an extra field is ignored', () => {
    expect(parseToOrb(stats({ fan_rpm: 1200 }))).toBeNull();
  });
});
