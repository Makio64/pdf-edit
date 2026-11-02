import assert from 'node:assert/strict'
import { describe, it } from 'node:test'
import { buildStats, countDay, MAX_TABLE_ROWS, MAX_VALUES_PER_DAY, parseRange, ROLLUP_GRACE_MS } from '../netlify/lib/aggregate.mts'
import { eventKey, parseDims, rollupKey } from '../netlify/lib/keys.mts'
import { memoryStore, PAGE_SIZE, type MemoryStore } from '../netlify/lib/memory-store.mts'
import { appendEvent, listKeysCapped } from '../netlify/lib/store.mts'

const NOW = Date.UTC(2026, 10, 1, 12, 0, 0) // 2026-11-01 12:00 UTC
const TODAY = '2026-11-01'
const YESTERDAY = '2026-10-31'

let seq = 0
function put(store: MemoryStore, day: string, n: string, dims: Record<string, string>, times = 1) {
  for (let i = 0; i < times; i++) {
    seq++
    store.data.set(eventKey(day, n, dims, Date.parse(`${day}T10:00:00Z`) + seq, `r${String(seq).padStart(7, '0')}`), '1')
  }
}

describe('parseRange', () => {
  it('accepts the dashboard ranges and defaults to 30', () => {
    assert.equal(parseRange('7'), 7)
    assert.equal(parseRange('90'), 90)
    assert.equal(parseRange('365'), 365)
    assert.equal(parseRange('all'), 'all')
    assert.equal(parseRange(null), 30)
    assert.equal(parseRange('31'), 30)
    assert.equal(parseRange('7; drop'), 30)
  })
})

describe('countDay', () => {
  it('counts keys per event and dims without reading values', async () => {
    const store = memoryStore()
    put(store, TODAY, 'pv', { p: '/', e: '1' }, 3)
    put(store, TODAY, 'pv', { p: '/app/', e: '0' })
    put(store, TODAY, 'pw', {})
    put(store, YESTERDAY, 'pv', { p: '/' })
    store.data.set(`e/${TODAY}/evil/-/x-y`, '1')
    const counts = await countDay(store, TODAY)
    assert.deepEqual(counts, { pv: { 'e=1&p=%2F': 3, 'e=0&p=%2Fapp%2F': 1 }, pw: { '-': 1 } })
    assert.equal(store.calls.get.length, 0)
  })
})

describe('limits against made-up values', () => {
  it('keeps the most frequent referrers and campaigns of a day, the others become other', async () => {
    const store = memoryStore()
    put(store, YESTERDAY, 'pv', { p: '/', e: '1', r: 'google', u: 'lettre' }, 50)
    for (let i = 0; i < MAX_VALUES_PER_DAY + 100; i++) put(store, YESTERDAY, 'pv', { p: '/', e: '1', r: `spam${i}.example`, u: `c${i}` })
    const counts = await countDay(store, YESTERDAY)
    const dims = Object.entries(counts.pv).map(([enc, c]) => [parseDims(enc), c] as const)
    const total = dims.reduce((s, [, c]) => s + c, 0)
    assert.equal(total, 50 + MAX_VALUES_PER_DAY + 100, 'nothing is lost')
    const referrers = new Set(dims.map(([d]) => d.r))
    const campaigns = new Set(dims.map(([d]) => d.u))
    assert.ok(referrers.size <= MAX_VALUES_PER_DAY + 1, `${referrers.size} referrers`)
    assert.ok(campaigns.size <= MAX_VALUES_PER_DAY + 1, `${campaigns.size} campaigns`)
    assert.ok(referrers.has('google') && referrers.has('other'))
    assert.ok(campaigns.has('lettre') && campaigns.has('other'))
  })

  it('gives at most MAX_TABLE_ROWS rows per breakdown, the rest in other', async () => {
    const store = memoryStore()
    put(store, TODAY, 'pv', { p: '/', e: '1', r: 'other' }, 3)
    for (let i = 0; i < MAX_TABLE_ROWS + 20; i++) put(store, TODAY, 'pv', { p: '/', e: '1', r: `site${String(i).padStart(3, '0')}.fr` }, i < 5 ? 10 : 1)
    const { breakdowns, totals } = await buildStats(store, { range: 7, nowMs: NOW })
    const rows = Object.entries(breakdowns.referrers)
    assert.equal(rows.length, MAX_TABLE_ROWS)
    assert.equal(rows.reduce((s, [, c]) => s + c, 0), totals.visits)
    assert.deepEqual(rows[0], ['other', 3 + (MAX_TABLE_ROWS + 20) - 5 - (MAX_TABLE_ROWS - 1 - 5)])
    assert.deepEqual(rows[1], ['site000.fr', 10])
    for (let i = 1; i < rows.length; i++) assert.ok(rows[i - 1][1] >= rows[i][1], 'sorted')
  })

  it('lists a day page by page and stops at the cap', async () => {
    const store = memoryStore()
    put(store, TODAY, 'pv', { p: '/' }, PAGE_SIZE * 2 + 500)
    const all = await listKeysCapped(store, `e/${TODAY}/`, 10 * PAGE_SIZE)
    assert.equal(all.keys.length, PAGE_SIZE * 2 + 500)
    assert.equal(all.truncated, false)
    store.calls.pages = 0
    const capped = await listKeysCapped(store, `e/${TODAY}/`, PAGE_SIZE + 10)
    assert.equal(capped.keys.length, PAGE_SIZE + 10)
    assert.equal(capped.truncated, true)
    assert.equal(store.calls.pages, 2, 'no page fetched after the cap')
  })
})

describe('buildStats', () => {
  it('fills every day of the range with zeros', async () => {
    const store = memoryStore()
    put(store, '2026-10-29', 'dl', { v: '1.1.1', src: 'hero', os: 'mac' }, 2)
    const stats = await buildStats(store, { range: 7, nowMs: NOW })
    assert.deepEqual(stats.range, { from: '2026-10-26', to: TODAY, days: 7 })
    assert.equal(stats.series.length, 7)
    assert.deepEqual(
      stats.series.map((d) => d.date),
      ['2026-10-26', '2026-10-27', '2026-10-28', '2026-10-29', '2026-10-30', '2026-10-31', '2026-11-01'],
    )
    assert.deepEqual(
      stats.series.map((d) => d.dl),
      [0, 0, 0, 2, 0, 0, 0],
    )
    assert.deepEqual(stats.series[0], { date: '2026-10-26', pv: 0, visits: 0, dl: 0, conv: 0, conv_ok: 0, eopen: 0, esave: 0, gh: 0 })
    assert.equal(stats.generatedAt, new Date(NOW).toISOString())
  })

  it('counts visits as page views with e=1', async () => {
    const store = memoryStore()
    put(store, TODAY, 'pv', { p: '/', e: '1', r: 'google', os: 'mac', d: 'desktop' }, 4)
    put(store, TODAY, 'pv', { p: '/app/', e: '0', os: 'mac', d: 'desktop' }, 6)
    put(store, TODAY, 'pv', { p: '/mentions-legales/' })
    const { totals, series, breakdowns } = await buildStats(store, { range: 30, nowMs: NOW })
    assert.equal(totals.pv, 11)
    assert.equal(totals.visits, 4)
    assert.equal(series.at(-1)!.visits, 4)
    assert.deepEqual(breakdowns.pages, { '/app/': 6, '/': 4, '/mentions-legales/': 1 })
    assert.deepEqual(breakdowns.referrers, { google: 4 })
    assert.deepEqual(breakdowns.os, { mac: 4 })
  })

  it('computes totals and sorted breakdowns', async () => {
    const store = memoryStore()
    put(store, TODAY, 'pv', { p: '/', e: '1', r: 'direct', u: 'lettre', d: 'mobile', os: 'ios' }, 2)
    put(store, TODAY, 'pv', { p: '/', e: '1', r: 'qwant', d: 'desktop', os: 'windows' }, 5)
    put(store, TODAY, 'conv', { f: 'docx', ok: '1', t: 'lt5' }, 3)
    put(store, TODAY, 'conv', { f: 'pages', ok: '1', t: '5-15' })
    put(store, TODAY, 'conv', { f: 'pages', ok: '0', err: 'other' }, 2)
    put(store, TODAY, 'pw', { w: 'conv' })
    put(store, TODAY, 'eopen', { ok: '1' }, 2)
    put(store, TODAY, 'esave', { ok: '1' }, 2)
    put(store, TODAY, 'esave', { ok: '0' })
    put(store, TODAY, 'gh', { src: 'footer' })
    put(store, YESTERDAY, 'dl', { v: '1.1.0', src: 'header', os: 'mac' })
    put(store, TODAY, 'dl', { v: '1.1.1', src: 'hero', os: 'mac' }, 3)
    put(store, TODAY, 'dl', { v: '1.1.1', src: 'other', os: 'windows' })
    const { totals, breakdowns } = await buildStats(store, { range: 7, nowMs: NOW })
    assert.deepEqual(totals, {
      pv: 7,
      visits: 7,
      dl: 5,
      conv: 6,
      conv_ok: 4,
      conv_fail: 2,
      pw: 1,
      eopen: 2,
      esave: 3,
      esave_ok: 2,
      gh: 1,
    })
    assert.deepEqual(Object.entries(breakdowns.referrers), [['qwant', 5], ['direct', 2]])
    assert.deepEqual(breakdowns.utm, { lettre: 2 })
    assert.deepEqual(breakdowns.device, { desktop: 5, mobile: 2 })
    assert.deepEqual(breakdowns.conv_format, { docx: 3, pages: 3 })
    assert.deepEqual(breakdowns.conv_time, { lt5: 3, '5-15': 1 })
    assert.deepEqual(breakdowns.conv_errors, { other: 2 })
    assert.deepEqual(Object.entries(breakdowns.dl_version), [['1.1.1', 4], ['1.1.0', 1]])
    assert.deepEqual(breakdowns.dl_src, { hero: 3, header: 1, other: 1 })
    assert.deepEqual(breakdowns.dl_os, { mac: 4, windows: 1 })
    assert.deepEqual(breakdowns.gh_src, { footer: 1 })
  })

  it('writes the rollup of a past day once and reuses it', async () => {
    const store = memoryStore()
    put(store, YESTERDAY, 'pv', { p: '/', e: '1' }, 2)
    put(store, TODAY, 'pv', { p: '/', e: '1' })

    const first = await buildStats(store, { range: 7, nowMs: NOW })
    assert.equal(first.series.at(-2)!.pv, 2)
    const rollup = JSON.parse(store.data.get(rollupKey(YESTERDAY))!)
    assert.deepEqual(rollup, { date: YESTERDAY, counts: { pv: { 'e=1&p=%2F': 2 } } })
    assert.deepEqual(
      store.calls.set.filter((k) => k.startsWith('r/')),
      [rollupKey(YESTERDAY)],
    )

    // A late raw event for yesterday is ignored: the frozen rollup wins.
    put(store, YESTERDAY, 'pv', { p: '/', e: '1' })
    // Today stays live.
    put(store, TODAY, 'pv', { p: '/', e: '1' }, 2)

    store.calls.list.length = 0
    const second = await buildStats(store, { range: 7, nowMs: NOW + 60_000 })
    assert.equal(second.series.at(-2)!.pv, 2)
    assert.equal(second.series.at(-1)!.pv, 3)
    assert.equal(store.calls.set.filter((k) => k.startsWith('r/')).length, 1, 'rollup written once')
    assert.ok(!store.calls.list.includes(`e/${YESTERDAY}/`), 'yesterday is not listed again')
    assert.ok(store.calls.list.includes(`e/${TODAY}/`), 'today is listed live')
  })

  it('never overwrites an existing rollup (onlyIfNew)', async () => {
    const store = memoryStore({ [rollupKey(YESTERDAY)]: JSON.stringify({ date: YESTERDAY, counts: { gh: { 'src=faq': 9 } } }) })
    put(store, YESTERDAY, 'gh', { src: 'faq' })
    const stats = await buildStats(store, { range: 7, nowMs: NOW })
    assert.equal(stats.totals.gh, 9)
    assert.equal(store.calls.set.length, 0)
  })

  it('does not freeze yesterday during the first minutes of the day', async () => {
    const store = memoryStore()
    put(store, YESTERDAY, 'pv', { p: '/' })
    const justAfterMidnight = Date.UTC(2026, 10, 1, 0, 0, 0) + ROLLUP_GRACE_MS - 1
    const stats = await buildStats(store, { range: 7, nowMs: justAfterMidnight })
    assert.equal(stats.series.at(-2)!.pv, 1)
    assert.equal(store.data.has(rollupKey(YESTERDAY)), false)
    await buildStats(store, { range: 7, nowMs: justAfterMidnight + 2 })
    assert.equal(store.data.has(rollupKey(YESTERDAY)), true)
  })

  it('never rolls up today', async () => {
    const store = memoryStore()
    put(store, TODAY, 'pv', { p: '/' })
    await buildStats(store, { range: 7, nowMs: Date.UTC(2026, 10, 1, 23, 59, 59) })
    assert.equal(store.data.has(rollupKey(TODAY)), false)
  })

  it('allTime sums every rollup plus today, beyond the selected range', async () => {
    const store = memoryStore({
      [rollupKey('2025-01-15')]: JSON.stringify({ date: '2025-01-15', counts: { dl: { 'v=1.0.0': 10 } } }),
    })
    put(store, '2026-06-01', 'dl', { v: '1.1.0' }, 5)
    put(store, YESTERDAY, 'dl', { v: '1.1.1' }, 2)
    put(store, TODAY, 'dl', { v: '1.1.1' }, 1)
    const stats = await buildStats(store, { range: 7, nowMs: NOW })
    assert.equal(stats.totals.dl, 3)
    assert.equal(stats.allTime.dl, 18)
    assert.ok(store.data.has(rollupKey('2026-06-01')), 'old days without a rollup are rolled up too')
  })

  it("range 'all' starts at the first day with data", async () => {
    const store = memoryStore({
      [rollupKey('2026-10-20')]: JSON.stringify({ date: '2026-10-20', counts: { pv: { 'p=%2F': 1 } } }),
    })
    put(store, '2026-10-25', 'pv', { p: '/' })
    const stats = await buildStats(store, { range: 'all', nowMs: NOW })
    assert.deepEqual(stats.range, { from: '2026-10-20', to: TODAY, days: 13 })
    assert.equal(stats.series.length, 13)
    assert.equal(stats.totals.pv, 2)
    assert.deepEqual(stats.totals, stats.allTime)
  })

  it("range 'all' on an empty store is just today", async () => {
    const stats = await buildStats(memoryStore(), { range: 'all', nowMs: NOW })
    assert.deepEqual(stats.range, { from: TODAY, to: TODAY, days: 1 })
    assert.equal(stats.allTime.pv, 0)
    assert.deepEqual(stats.breakdowns.pages, {})
  })

  it('reads what appendEvent writes', async () => {
    const store = memoryStore()
    await appendEvent(store, { n: 'conv', dims: { f: 'docx', ok: '1', t: 'gt60' } }, NOW)
    const stats = await buildStats(store, { range: 7, nowMs: NOW })
    assert.equal(stats.totals.conv_ok, 1)
    assert.deepEqual(stats.breakdowns.conv_time, { gt60: 1 })
  })
})
