// The only part that knows about Netlify Blobs' shape. Everything else takes a
// `StatsStore`, so tests can pass an in-memory fake.

import { dayFromPath, dayPrefix, eventKey, utcDay, EVENT_PREFIX, ROLLUP_PREFIX } from './keys.mts'
import type { ValidEvent } from './validate.mts'

export interface ListResult {
  blobs: { key: string }[]
  directories: string[]
}

export interface ListOptions {
  prefix?: string
  directories?: boolean
}

export interface StatsStore {
  set(key: string, value: string, options?: { onlyIfNew?: boolean }): Promise<unknown>
  get(key: string, options: { type: 'json' }): Promise<unknown>
  list(options: ListOptions & { paginate: true }): AsyncIterable<ListResult>
  list(options: ListOptions & { paginate?: false }): Promise<ListResult>
}

/**
 * At most this many raw events are counted for one day. Far above this site's traffic: it only
 * bounds the time /api/stats spends listing a day that someone flooded with events.
 */
export const MAX_KEYS_PER_DAY = 50_000

/** The counts stay in Europe (Frankfurt), next to the functions (config.region 'fra'). A site-wide
 * store must name its region on every call, or it silently reads and writes another one (us-east-2). */
export const STATS_REGION = 'eu-central-1'

/** `stats` in production; `stats-dev`, `stats-deploy-preview`, … elsewhere, so tests never pollute production. */
export function storeName(deployContext: string | null | undefined): string {
  const ctx = (deployContext ?? '').trim() || 'dev'
  if (ctx === 'production') return 'stats'
  return `stats-${ctx.toLowerCase().replace(/[^a-z0-9-]+/g, '-')}`.slice(0, 64)
}

export async function appendEvent(store: StatsStore, event: ValidEvent, nowMs: number): Promise<string> {
  const key = eventKey(utcDay(nowMs), event.n, event.dims, nowMs)
  await store.set(key, '1')
  return key
}

export async function listKeys(store: StatsStore, prefix: string): Promise<string[]> {
  const { blobs } = await store.list({ prefix })
  return blobs.map((b) => b.key)
}

/** Days that have at least one raw event (from the `e/<day>` directories). */
export async function listEventDays(store: StatsStore): Promise<Set<string>> {
  const { blobs, directories } = await store.list({ prefix: EVENT_PREFIX, directories: true })
  const days = new Set<string>()
  for (const dir of directories) {
    const day = dayFromPath(dir)
    if (day) days.add(day)
  }
  // Defensive: a backend that ignores `directories` returns every key instead.
  for (const { key } of blobs) {
    const day = dayFromPath(key)
    if (day) days.add(day)
  }
  return days
}

export async function listRollupDays(store: StatsStore): Promise<Set<string>> {
  const days = new Set<string>()
  for (const key of await listKeys(store, ROLLUP_PREFIX)) {
    const day = dayFromPath(key)
    if (day) days.add(day)
  }
  return days
}

/** The keys under `prefix`, page by page, stopping after `max` keys (`truncated` then). */
export async function listKeysCapped(
  store: StatsStore,
  prefix: string,
  max: number,
): Promise<{ keys: string[]; truncated: boolean }> {
  const keys: string[] = []
  // Netlify Blobs returns the async iterable itself; `await` also accepts a backend that answers
  // with a promise of it, or of one plain page.
  const result: unknown = await store.list({ prefix, paginate: true })
  const pages = Array.isArray((result as ListResult | null)?.blobs) ? [result as ListResult] : (result as AsyncIterable<ListResult>)
  for await (const page of pages) {
    for (const { key } of page.blobs) {
      if (keys.length >= max) return { keys, truncated: true }
      keys.push(key)
    }
  }
  return { keys, truncated: false }
}

/** The raw event keys of one day, at most MAX_KEYS_PER_DAY of them. */
export function listDayKeys(store: StatsStore, day: string, max = MAX_KEYS_PER_DAY): Promise<{ keys: string[]; truncated: boolean }> {
  return listKeysCapped(store, dayPrefix(day), max)
}

/** Runs `fn` over `items` with at most `limit` calls in flight. */
export async function mapLimit<T, R>(items: T[], limit: number, fn: (item: T) => Promise<R>): Promise<R[]> {
  const out: R[] = new Array(items.length)
  let next = 0
  const worker = async () => {
    while (next < items.length) {
      const i = next++
      out[i] = await fn(items[i])
    }
  }
  await Promise.all(Array.from({ length: Math.min(limit, items.length) }, worker))
  return out
}
