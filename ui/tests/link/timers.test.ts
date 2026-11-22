import { describe, expect, test } from 'vitest';
import { encodeFromOrb, MAX_TIMER_LABEL_CHARS, parseToOrb, type FromOrb } from '../../src/link/protocol';

// The same cases, in the same order, as crates/orb-protocol/tests/timer_tests.rs.
const oneTimer = (label: string) =>
  `{"type":"timers","timers":[{"id":1,"label":"${label}","ends_at_ms":2,"state":"running"}]}`;

const VALID_TO_ORB: readonly string[] = [
  '{"type":"timers","timers":[]}',
  '{"type":"timers","timers":[{"id":3,"label":"Time to sleep","ends_at_ms":1763830800000,"state":"running"}]}',
  '{"type":"timers","timers":[{"id":0,"ends_at_ms":0,"state":"pending"}]}',
  '{"type":"timers","timers":[{"id":1,"label":"","ends_at_ms":2,"state":"ended"}]}',
  '{"type":"timers","timers":[{"id":9007199254740991,"ends_at_ms":9007199254740991,"state":"running"}]}',
  '{"type":"timers","timers":[{"state":"running","ends_at_ms":2,"label":"Reordered","id":1}]}',
  oneTimer('a'.repeat(60)),
  oneTimer('\u{1f600}'.repeat(60)),
];

const INVALID_TO_ORB: readonly string[] = [
  '{"type":"timers"}',
  '{"type":"timers","timers":null}',
  '{"type":"timers","timers":{}}',
  '{"type":"timers","timers":[],"extra":1}',
  '{"type":"timers","timers":[{"id":1,"ends_at_ms":2,"state":"cancelled"}]}',
  '{"type":"timers","timers":[{"id":1,"ends_at_ms":2,"state":"Running"}]}',
  '{"type":"timers","timers":[{"id":1,"ends_at_ms":2,"state":null}]}',
  '{"type":"timers","timers":[{"id":1,"ends_at_ms":2,"state":"running","extra":1}]}',
  '{"type":"timers","timers":[{"ends_at_ms":2,"state":"running"}]}',
  '{"type":"timers","timers":[{"id":1,"state":"running"}]}',
  '{"type":"timers","timers":[{"id":1,"ends_at_ms":2}]}',
  '{"type":"timers","timers":[{"id":-1,"ends_at_ms":2,"state":"running"}]}',
  '{"type":"timers","timers":[{"id":1.5,"ends_at_ms":2,"state":"running"}]}',
  '{"type":"timers","timers":[{"id":"1","ends_at_ms":2,"state":"running"}]}',
  '{"type":"timers","timers":[{"id":1,"ends_at_ms":-2,"state":"running"}]}',
  '{"type":"timers","timers":[{"id":1,"ends_at_ms":1.5,"state":"running"}]}',
  '{"type":"timers","timers":[{"id":1,"ends_at_ms":"2","state":"running"}]}',
  '{"type":"timers","timers":[{"id":1,"label":null,"ends_at_ms":2,"state":"running"}]}',
  '{"type":"timers","timers":[{"id":1,"label":3,"ends_at_ms":2,"state":"running"}]}',
  '{"type":"timers","timers":[3]}',
  oneTimer('a'.repeat(61)),
  oneTimer('\u{1f600}'.repeat(61)),
];

const named = (text: string): [string, string] => [text.length > 120 ? `${text.slice(0, 117)}...` : text, text];

describe('the timers message', () => {
  test('the label limit matches the Rust side', () => {
    expect(MAX_TIMER_LABEL_CHARS).toBe(60);
  });

  test.each(VALID_TO_ORB.map(named))('%s parses', (_name, text) => {
    expect(parseToOrb(text)).not.toBeNull();
  });

  test.each(INVALID_TO_ORB.map(named))('%s is ignored', (_name, text) => {
    expect(parseToOrb(text)).toBeNull();
  });

  test('a timer keeps its fields, with no label as null and an empty list as an empty list', () => {
    expect(
      parseToOrb(
        '{"type":"timers","timers":[{"id":3,"label":"Time to sleep","ends_at_ms":1763830800000,"state":"running"},{"id":4,"ends_at_ms":5,"state":"pending"},{"id":5,"label":"","ends_at_ms":6,"state":"ended"}]}',
      ),
    ).toEqual({
      type: 'timers',
      timers: [
        { id: 3, label: 'Time to sleep', endsAtMs: 1763830800000, state: 'running' },
        { id: 4, label: null, endsAtMs: 5, state: 'pending' },
        { id: 5, label: '', endsAtMs: 6, state: 'ended' },
      ],
    });
    expect(parseToOrb('{"type":"timers","timers":[]}')).toEqual({ type: 'timers', timers: [] });
  });

  test('a message whose timers include one bad timer is ignored whole', () => {
    expect(
      parseToOrb(
        '{"type":"timers","timers":[{"id":1,"ends_at_ms":2,"state":"running"},{"id":2,"ends_at_ms":2,"state":"cancelled"}]}',
      ),
    ).toBeNull();
  });
});

describe('the timer answers', () => {
  const cases: readonly (readonly [FromOrb, string])[] = [
    [{ type: 'cancel_timer', id: 3 }, '{"type":"cancel_timer","id":3}'],
    [{ type: 'confirm_timer', id: 3 }, '{"type":"confirm_timer","id":3}'],
    [{ type: 'reject_timer', id: 3 }, '{"type":"reject_timer","id":3}'],
  ];

  test.each(cases)('%o is encoded as the Rust side expects', (message, text) => {
    expect(encodeFromOrb(message)).toBe(text);
  });
});
