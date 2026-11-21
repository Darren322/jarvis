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
    ['{"type":"show_panel","panel":"camera"}', { type: 'show_panel', panel: 'camera' }],
    ['{"type":"hide_panel"}', { type: 'hide_panel' }],
  ])('%s parses', (text, message) => {
    expect(parseToOrb(text)).toEqual(message);
  });

  test.each([
    '{"type":"show_panel","panel":"weather"}',
    '{"type":"show_panel","panel":"Stats"}',
    '{"type":"show_panel","panel":"Camera"}',
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

describe('the built Panel', () => {
  // The wire examples from crates/orb-protocol/tests/built_panel_tests.rs.
  test.each([
    [
      '{"type":"show_built_panel","title":"Facts","parts":[{"kind":"list","title":"Facts","items":["Darren likes tea","Darren lives in Penang"]}]}',
      { title: 'Facts', parts: [{ kind: 'list', title: 'Facts', items: ['Darren likes tea', 'Darren lives in Penang'] }] },
    ],
    [
      '{"type":"show_built_panel","title":"Note","parts":[{"kind":"card","title":"Reminder","text":"Water the plants"}]}',
      { title: 'Note', parts: [{ kind: 'card', title: 'Reminder', text: 'Water the plants' }] },
    ],
    [
      '{"type":"show_built_panel","title":"Pi","parts":[{"kind":"gauge","source":"cpu"},{"kind":"gauge","source":"temperature"},{"kind":"gauge","source":"ram"},{"kind":"gauge","source":"storage"}]}',
      {
        title: 'Pi',
        parts: [
          { kind: 'gauge', source: 'cpu' },
          { kind: 'gauge', source: 'temperature' },
          { kind: 'gauge', source: 'ram' },
          { kind: 'gauge', source: 'storage' },
        ],
      },
    ],
    [
      '{"type":"show_built_panel","title":"Servers","parts":[{"kind":"gauge","source":"llama"},{"kind":"gauge","source":"bge"}]}',
      { title: 'Servers', parts: [{ kind: 'gauge', source: 'llama' }, { kind: 'gauge', source: 'bge' }] },
    ],
  ])('%s parses', (text, panel) => {
    expect(parseToOrb(text)).toEqual({ type: 'show_built_panel', panel });
  });

  const message = (parts: string, title = '"T"') => `{"type":"show_built_panel","title":${title},"parts":[${parts}]}`;

  test.each([
    // Not in the Catalog.
    message('{"kind":"chart","title":"x"}'),
    message('{"kind":"timer","seconds":600}'),
    message('{"kind":"Card","title":"a","text":"b"}'),
    message('{"title":"a","text":"b"}'),
    message('"card"'),
    message('null'),
    // Unknown gauge sources, and a model writing a gauge's number.
    message('{"kind":"gauge","source":"weather"}'),
    message('{"kind":"gauge","source":"CPU"}'),
    message('{"kind":"gauge"}'),
    message('{"kind":"gauge","source":"cpu","value":12}'),
    // Unknown or missing fields in a part.
    message('{"kind":"card","title":"a","text":"b","html":"<b>"}'),
    message('{"kind":"card","title":"a"}'),
    message('{"kind":"list","title":"a","items":["b"],"extra":1}'),
    message('{"kind":"list","title":"a"}'),
    message('{"kind":"list","title":"a","items":[1]}'),
    message('{"kind":"list","title":"a","items":"b"}'),
    // Unknown or missing fields in the message.
    '{"type":"show_built_panel","title":"T","parts":[{"kind":"gauge","source":"cpu"}],"extra":1}',
    '{"type":"show_built_panel","parts":[{"kind":"gauge","source":"cpu"}]}',
    '{"type":"show_built_panel","title":"T"}',
    message('{"kind":"gauge","source":"cpu"}', 'null'),
    // The built Panel is never shown by name.
    '{"type":"show_panel","panel":"built"}',
  ])('%s is ignored', (text) => {
    expect(parseToOrb(text)).toBeNull();
  });

  const text = (count: number, unit = 'a') => JSON.stringify(unit.repeat(count));
  const gauges = (count: number) => Array.from({ length: count }, () => '{"kind":"gauge","source":"cpu"}').join(',');
  const items = (count: number) => Array.from({ length: count }, () => '"milk"').join(',');

  // The limits of crates/orb-protocol at their edges, as literals (jarvis-ui 12, G4).
  // 🍎 is two UTF-16 units: counting `length` would reject its accepted row.
  test.each([
    ['0 parts', message(gauges(0)), false],
    ['1 part', message(gauges(1)), true],
    ['4 parts', message(gauges(4)), true],
    ['5 parts', message(gauges(5)), false],
    ['0 list items', message(`{"kind":"list","title":"a","items":[${items(0)}]}`), false],
    ['8 list items', message(`{"kind":"list","title":"a","items":[${items(8)}]}`), true],
    ['9 list items', message(`{"kind":"list","title":"a","items":[${items(9)}]}`), false],
    ['card title of 120', message(`{"kind":"card","title":${text(120)},"text":"b"}`), true],
    ['card title of 121', message(`{"kind":"card","title":${text(121)},"text":"b"}`), false],
    ['card text of 120', message(`{"kind":"card","title":"a","text":${text(120)}}`), true],
    ['card text of 121', message(`{"kind":"card","title":"a","text":${text(121)}}`), false],
    ['card text of 120 🍎', message(`{"kind":"card","title":"a","text":${text(120, '🍎')}}`), true],
    ['card text of 121 🍎', message(`{"kind":"card","title":"a","text":${text(121, '🍎')}}`), false],
    ['list title of 120', message(`{"kind":"list","title":${text(120)},"items":["b"]}`), true],
    ['list title of 121', message(`{"kind":"list","title":${text(121)},"items":["b"]}`), false],
    ['list item of 120', message(`{"kind":"list","title":"a","items":["b",${text(120)}]}`), true],
    ['list item of 121', message(`{"kind":"list","title":"a","items":["b",${text(121)}]}`), false],
    ['panel title of 40', message(gauges(1), text(40)), true],
    ['panel title of 41', message(gauges(1), text(41)), false],
  ])('%s', (_, line, accepted) => {
    expect(parseToOrb(line) !== null).toBe(accepted);
  });
});
