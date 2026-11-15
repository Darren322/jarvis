import { expect, test } from 'vitest';
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
  '{"type":"show_panel","panel":"stats"}',
  '{"type":"State","state":"idle"}',
  '{"type":"state","state":"Idle"}',
  '{"type":"level","level":"0.5","source":"mic"}',
  '{"type":"level","level":0.5,"source":"mic","extra":true}',
  '{"type":"tap"}',
])('%s is ignored', (text) => {
  expect(parseToOrb(text)).toBeNull();
});

test('a Tap is encoded as the Rust side expects', () => {
  expect(encodeFromOrb({ type: 'tap' })).toBe('{"type":"tap"}');
});
