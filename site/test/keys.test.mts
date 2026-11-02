import assert from 'node:assert/strict'
import { describe, it } from 'node:test'
import {
  addDays,
  dayFromPath,
  daysBetween,
  encodeDims,
  eventKey,
  parseDims,
  parseEventKey,
  rollupKey,
  utcDay,
} from '../netlify/lib/keys.mts'

describe('encodeDims / parseDims', () => {
  it('sorts by key and joins k=v with &', () => {
    assert.equal(encodeDims({ p: '/', e: '1', d: 'desktop' }), 'd=desktop&e=1&p=%2F')
  })
  it('uses - when there are no dims', () => {
    assert.equal(encodeDims({}), '-')
    assert.deepEqual(parseDims('-'), {})
    assert.deepEqual(parseDims(''), {})
  })

  const tricky: Record<string, string>[] = [
    { a: 'x&y=z' },
    { path: '/app/edit.html', q: 'a/b/c' },
    { pct: '100%', twice: '%25' },
    { u: 'é à ç ü — 日本語 🙂' },
    { eq: '==', amp: '&&', both: '&=&=' },
    { space: 'a b+c', plus: '+' },
    { dash: '-', empty: '' },
    { 'k&y': 'v', 'k=2': 'w' },
  ]
  for (const dims of tricky) {
    it(`round-trips ${JSON.stringify(dims)}`, () => {
      const encoded = encodeDims(dims)
      assert.ok(!encoded.includes('/'), 'never contains a path separator')
      assert.deepEqual(parseDims(encoded), dims)
      const key = eventKey('2026-11-01', 'pv', dims, 1_700_000_000_000, 'abcd1234')
      assert.deepEqual(parseEventKey(key), { day: '2026-11-01', n: 'pv', dims: encoded })
      assert.deepEqual(parseDims(parseEventKey(key)!.dims), dims)
    })
  }

  it('is independent of insertion order', () => {
    assert.equal(encodeDims({ b: '2', a: '1' }), encodeDims({ a: '1', b: '2' }))
  })
  it('ignores malformed pairs instead of throwing', () => {
    assert.deepEqual(parseDims('a=%E0%A4%A&b=2&noeq&=x'), { b: '2' })
  })
})

describe('eventKey / parseEventKey', () => {
  it('builds e/<day>/<n>/<dims>/<base36 time>-<8 random>', () => {
    const now = Date.UTC(2026, 10, 1, 12)
    const key = eventKey('2026-11-01', 'conv', { ok: '1', f: 'docx' }, now)
    const match = /^e\/2026-11-01\/conv\/f=docx&ok=1\/([0-9a-z]+)-([0-9a-z]{8})$/.exec(key)
    assert.ok(match, key)
    assert.equal(parseInt(match[1], 36), now)
  })
  it('gives different keys to simultaneous events', () => {
    const keys = new Set(Array.from({ length: 200 }, () => eventKey('2026-11-01', 'pv', { p: '/' }, 1)))
    assert.equal(keys.size, 200)
  })
  it('reads a key whose %2F came back decoded as / the same way', () => {
    const dims = { d: 'desktop', e: '1', os: 'mac', p: '/app/edit.html', r: 'direct' }
    const key = eventKey('2026-11-01', 'pv', dims, 1_700_000_000_000, 'abcd1234')
    const decoded = decodeURIComponent(key)
    assert.notEqual(decoded, key)
    assert.deepEqual(parseEventKey(decoded), parseEventKey(key))
    assert.deepEqual(parseDims(parseEventKey(decoded)!.dims), dims)
    const home = decodeURIComponent(eventKey('2026-11-01', 'pv', { p: '/', r: 'google' }, 1, 'abcd1234'))
    assert.deepEqual(parseEventKey(home), { day: '2026-11-01', n: 'pv', dims: 'p=%2F&r=google' })
  })
  it('rejects keys that are not event keys', () => {
    for (const key of [
      'r/2026-11-01',
      'c/github',
      'e/2026-11-01/pv/p=%2F',
      'e/not-a-day/pv/p=%2F/x-y',
      'e/2026-13-45/pv/-/x-y',
      'e/2026-11-01/pv/garbage/x-y',
      'e/2026-11-01/pv/p=%2F/x-y/extra',
      'e/2026-11-01/pv/p=%2F/no_dash',
      'e/2026-11-01/pv/p=%2F/',
    ]) {
      assert.equal(parseEventKey(key), null, key)
    }
  })
})

describe('days', () => {
  it('utcDay uses UTC', () => {
    assert.equal(utcDay(Date.UTC(2026, 10, 1, 23, 59, 59)), '2026-11-01')
    assert.equal(utcDay(Date.UTC(2026, 10, 2, 0, 0, 0)), '2026-11-02')
  })
  it('addDays crosses months and years', () => {
    assert.equal(addDays('2026-03-01', -1), '2026-02-28')
    assert.equal(addDays('2026-12-31', 1), '2027-01-01')
    assert.equal(addDays('2026-11-01', -29), '2026-10-03')
  })
  it('daysBetween includes both ends', () => {
    assert.deepEqual(daysBetween('2026-10-30', '2026-11-02'), ['2026-10-30', '2026-10-31', '2026-11-01', '2026-11-02'])
    assert.deepEqual(daysBetween('2026-11-01', '2026-11-01'), ['2026-11-01'])
  })
  it('dayFromPath reads directories and rollup keys', () => {
    assert.equal(dayFromPath('e/2026-11-01'), '2026-11-01')
    assert.equal(dayFromPath('e/2026-11-01/'), '2026-11-01')
    assert.equal(dayFromPath(rollupKey('2026-10-31')), '2026-10-31')
    assert.equal(dayFromPath('c/github'), null)
  })
})
