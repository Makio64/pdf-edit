// Counting = listing the keys under a day prefix and parsing them (values are never read).
// Days strictly before today are rolled up once into `r/<day>` (written with onlyIfNew) and reused.

import { addDays, dayStartMs, daysBetween, encodeDims, parseDims, parseEventKey, rollupKey, utcDay } from './keys.mts'
import { listDayKeys, listEventDays, listRollupDays, mapLimit, type StatsStore } from './store.mts'
import { EVENTS, type Dims } from './validate.mts'

/** event name → encoded dims → count */
export type Counts = Record<string, Record<string, number>>

export interface Rollup {
  date: string
  counts: Counts
  /** Present when the day had more than MAX_KEYS_PER_DAY raw events: only the first ones were counted. */
  truncated?: true
}

export interface Totals {
  pv: number
  visits: number
  dl: number
  conv: number
  conv_ok: number
  conv_fail: number
  pw: number
  eopen: number
  esave: number
  esave_ok: number
  gh: number
}

export interface DayPoint {
  date: string
  pv: number
  visits: number
  dl: number
  conv: number
  conv_ok: number
  eopen: number
  esave: number
  gh: number
}

export const BREAKDOWNS = [
  'pages',
  'languages',
  'referrers',
  'utm',
  'os',
  'device',
  'conv_format',
  'conv_time',
  'conv_errors',
  'dl_version',
  'dl_src',
  'dl_lang',
  'dl_os',
  'gh_src',
] as const
export type BreakdownName = (typeof BREAKDOWNS)[number]
export type Breakdowns = Record<BreakdownName, Record<string, number>>

export type Range = 7 | 30 | 90 | 365 | 'all'

export interface Stats {
  range: { from: string; to: string; days: number }
  generatedAt: string
  series: DayPoint[]
  totals: Totals
  allTime: Totals
  breakdowns: Breakdowns
}

const DAY_MS = 86_400_000
/** Events are written with waitUntil just after midnight: wait a little before freezing yesterday. */
export const ROLLUP_GRACE_MS = 5 * 60_000
const CONCURRENCY = 16

export function parseRange(value: string | null | undefined): Range {
  if (value === 'all') return 'all'
  const n = Number(value)
  return n === 7 || n === 30 || n === 90 || n === 365 ? n : 30
}

/**
 * Dimensions whose values the browser chooses freely (any referring host, any utm_source): per
 * day, only the most frequent ones are kept and the others are counted as `other`, so neither a
 * rollup nor /api/stats can be blown up with millions of made-up values.
 */
export const OPEN_DIMS: Record<string, readonly string[]> = { pv: ['r', 'u'] }
export const MAX_VALUES_PER_DAY = 200
/** Rows per breakdown table in /api/stats; the rest is added to `other`. */
export const MAX_TABLE_ROWS = 50
export const OTHER = 'other'

function byCountThenName(a: [string, number], b: [string, number]): number {
  return b[1] - a[1] || (a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : 0)
}

/** Folds the rarer values of one open dimension into `other`. */
function capDimension(byDims: Record<string, number>, dim: string, max: number): Record<string, number> {
  const perValue = new Map<string, number>()
  for (const [encoded, c] of Object.entries(byDims)) {
    const value = dimsOf(encoded)[dim]
    if (value !== undefined && value !== OTHER) perValue.set(value, (perValue.get(value) ?? 0) + c)
  }
  if (perValue.size <= max) return byDims
  const keep = new Set([...perValue.entries()].sort(byCountThenName).slice(0, max).map(([value]) => value))
  const out: Record<string, number> = {}
  for (const [encoded, c] of Object.entries(byDims)) {
    const dims = dimsOf(encoded)
    const value = dims[dim]
    const key = value === undefined || value === OTHER || keep.has(value) ? encoded : encodeDims({ ...dims, [dim]: OTHER })
    out[key] = (out[key] ?? 0) + c
  }
  return out
}

export function capOpenDims(counts: Counts, max = MAX_VALUES_PER_DAY): Counts {
  for (const [n, dims] of Object.entries(OPEN_DIMS)) {
    if (!counts[n]) continue
    for (const dim of dims) counts[n] = capDimension(counts[n], dim, max)
  }
  return counts
}

/** Count of one day from its raw event keys (at most MAX_KEYS_PER_DAY of them). */
export async function countDayDetailed(store: StatsStore, day: string): Promise<{ counts: Counts; truncated: boolean }> {
  const counts: Counts = {}
  const { keys, truncated } = await listDayKeys(store, day)
  for (const key of keys) {
    const parsed = parseEventKey(key)
    if (!parsed || parsed.day !== day || !Object.hasOwn(EVENTS, parsed.n)) continue
    const byDims = (counts[parsed.n] ??= {})
    byDims[parsed.dims] = (byDims[parsed.dims] ?? 0) + 1
  }
  return { counts: capOpenDims(counts), truncated }
}

/** Live count of one day from its raw event keys. */
export async function countDay(store: StatsStore, day: string): Promise<Counts> {
  return (await countDayDetailed(store, day)).counts
}

function isRollup(value: unknown, day: string): value is Rollup {
  if (!value || typeof value !== 'object') return false
  const r = value as Partial<Rollup>
  return r.date === day && !!r.counts && typeof r.counts === 'object'
}

/** A finished day: its rollup when present, otherwise counted now and stored once. */
export async function pastDayCounts(
  store: StatsStore,
  day: string,
  { hasRollup, nowMs }: { hasRollup: boolean; nowMs: number },
): Promise<Counts> {
  if (hasRollup) {
    const stored = await store.get(rollupKey(day), { type: 'json' })
    if (isRollup(stored, day)) return stored.counts
  }
  const { counts, truncated } = await countDayDetailed(store, day)
  if (day < utcDay(nowMs) && nowMs >= dayStartMs(day) + DAY_MS + ROLLUP_GRACE_MS) {
    const rollup: Rollup = truncated ? { date: day, counts, truncated: true } : { date: day, counts }
    await store.set(rollupKey(day), JSON.stringify(rollup), { onlyIfNew: true })
  }
  return counts
}

export function emptyTotals(): Totals {
  return { pv: 0, visits: 0, dl: 0, conv: 0, conv_ok: 0, conv_fail: 0, pw: 0, eopen: 0, esave: 0, esave_ok: 0, gh: 0 }
}

function emptyBreakdowns(): Breakdowns {
  return Object.fromEntries(BREAKDOWNS.map((name) => [name, {}])) as Breakdowns
}

const dimsCache = new Map<string, Dims>()
function dimsOf(encoded: string): Dims {
  let dims = dimsCache.get(encoded)
  if (!dims) {
    dims = parseDims(encoded)
    if (dimsCache.size > 5000) dimsCache.clear()
    dimsCache.set(encoded, dims)
  }
  return dims
}

function bump(table: Record<string, number>, value: string | undefined, by: number, fallback = 'unknown') {
  const key = value ?? fallback
  table[key] = (table[key] ?? 0) + by
}

/** Adds one day's counts to totals and, when given, to the breakdown tables. */
export function accumulate(counts: Counts, totals: Totals, breakdowns?: Breakdowns): void {
  for (const [n, byDims] of Object.entries(counts)) {
    for (const [encoded, c] of Object.entries(byDims)) {
      if (typeof c !== 'number' || !(c > 0)) continue
      const d = dimsOf(encoded)
      const b = breakdowns
      switch (n) {
        case 'pv': {
          totals.pv += c
          if (b) {
            bump(b.pages, d.p, c)
            // Every page view, by the page's language (`unknown`: counted before languages, or an app page).
            bump(b.languages, d.l, c)
          }
          if (d.e === '1') {
            totals.visits += c
            if (b) {
              bump(b.referrers, d.r, c)
              if (d.u) bump(b.utm, d.u, c)
              bump(b.os, d.os, c)
              bump(b.device, d.d, c)
            }
          }
          break
        }
        case 'conv': {
          totals.conv += c
          if (d.ok === '1') totals.conv_ok += c
          else if (d.ok === '0') totals.conv_fail += c
          if (b) {
            bump(b.conv_format, d.f, c)
            if (d.ok === '1') bump(b.conv_time, d.t, c)
            if (d.ok === '0') bump(b.conv_errors, d.err, c)
          }
          break
        }
        case 'dl': {
          totals.dl += c
          if (b) {
            bump(b.dl_version, d.v, c)
            bump(b.dl_src, d.src, c, 'other')
            // The language of the page the link was on (`unknown`: a link without it, or before November 2026).
            bump(b.dl_lang, d.l, c)
            bump(b.dl_os, d.os, c)
          }
          break
        }
        case 'pw':
          totals.pw += c
          break
        case 'eopen':
          totals.eopen += c
          break
        case 'esave':
          totals.esave += c
          if (d.ok === '1') totals.esave_ok += c
          break
        case 'gh':
          totals.gh += c
          if (b) bump(b.gh_src, d.src, c, 'other')
          break
      }
    }
  }
}

/** Sorted by count (desc), at most MAX_TABLE_ROWS rows: the smaller ones are added to `other`. */
function sortedTable(table: Record<string, number>, max = MAX_TABLE_ROWS): Record<string, number> {
  const entries = Object.entries(table).sort(byCountThenName)
  if (entries.length <= max) return Object.fromEntries(entries)
  let other = table[OTHER] ?? 0
  const kept = entries.filter(([value]) => value !== OTHER)
  for (const [, c] of kept.splice(max - 1)) other += c
  kept.push([OTHER, other])
  return Object.fromEntries(kept.sort(byCountThenName))
}

function point(date: string, t: Totals): DayPoint {
  return {
    date,
    pv: t.pv,
    visits: t.visits,
    dl: t.dl,
    conv: t.conv,
    conv_ok: t.conv_ok,
    eopen: t.eopen,
    esave: t.esave,
    gh: t.gh,
  }
}

/** The whole /api/stats payload except `github`. */
export async function buildStats(store: StatsStore, { range, nowMs }: { range: Range; nowMs: number }): Promise<Stats> {
  const today = utcDay(nowMs)
  const [eventDays, rollupDays] = await Promise.all([listEventDays(store), listRollupDays(store)])
  const past = [...new Set([...eventDays, ...rollupDays])].filter((d) => d < today).sort()

  const byDay = new Map<string, Counts>()
  await mapLimit(past, CONCURRENCY, async (day) => {
    byDay.set(day, await pastDayCounts(store, day, { hasRollup: rollupDays.has(day), nowMs }))
  })
  byDay.set(today, await countDay(store, today))

  const from = range === 'all' ? (past[0] ?? today) : addDays(today, -(range - 1))
  const totals = emptyTotals()
  const breakdowns = emptyBreakdowns()
  const series = daysBetween(from, today).map((date) => {
    const counts = byDay.get(date) ?? {}
    const day = emptyTotals()
    accumulate(counts, day)
    accumulate(counts, totals, breakdowns)
    return point(date, day)
  })

  const allTime = emptyTotals()
  for (const counts of byDay.values()) accumulate(counts, allTime)

  return {
    range: { from, to: today, days: series.length },
    generatedAt: new Date(nowMs).toISOString(),
    series,
    totals,
    allTime,
    breakdowns: Object.fromEntries(BREAKDOWNS.map((name) => [name, sortedTable(breakdowns[name])])) as Breakdowns,
  }
}
