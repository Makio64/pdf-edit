// Blob key layout of the analytics store.
//   e/<YYYY-MM-DD UTC>/<n>/<dims>/<Date.now() base36>-<8 random base36 chars>   one blob per event, value "1"
//   r/<YYYY-MM-DD>                                                                daily rollup (JSON), written once
//   c/github                                                                      GitHub release downloads cache

import type { Dims } from './validate.mts'

export const EVENT_PREFIX = 'e/'
export const ROLLUP_PREFIX = 'r/'
export const GITHUB_CACHE_KEY = 'c/github'
export const EMPTY_DIMS = '-'

const DAY_RE = /^\d{4}-\d{2}-\d{2}$/
const DAY_MS = 86_400_000

export function isDay(value: string): boolean {
  return DAY_RE.test(value) && !Number.isNaN(Date.parse(`${value}T00:00:00Z`))
}

/** UTC calendar day of a timestamp, as YYYY-MM-DD. */
export function utcDay(ms: number): string {
  return new Date(ms).toISOString().slice(0, 10)
}

export function dayStartMs(day: string): number {
  return Date.parse(`${day}T00:00:00Z`)
}

export function addDays(day: string, delta: number): string {
  return utcDay(dayStartMs(day) + delta * DAY_MS)
}

/** Every day from `from` to `to`, both included. */
export function daysBetween(from: string, to: string): string[] {
  const out: string[] = []
  for (let ms = dayStartMs(from), end = dayStartMs(to); ms <= end; ms += DAY_MS) out.push(utcDay(ms))
  return out
}

/** Dimensions sorted by key, `k=v` joined by `&`, each value URI-encoded; `-` when empty. */
export function encodeDims(dims: Dims): string {
  const keys = Object.keys(dims).sort()
  if (!keys.length) return EMPTY_DIMS
  return keys.map((k) => `${encodeURIComponent(k)}=${encodeURIComponent(dims[k])}`).join('&')
}

export function parseDims(segment: string): Dims {
  const dims: Dims = {}
  if (!segment || segment === EMPTY_DIMS) return dims
  for (const pair of segment.split('&')) {
    const eq = pair.indexOf('=')
    if (eq <= 0) continue
    try {
      dims[decodeURIComponent(pair.slice(0, eq))] = decodeURIComponent(pair.slice(eq + 1))
    } catch {
      // A malformed escape can only come from a key we did not write: ignore that pair.
    }
  }
  return dims
}

function randomBase36(length: number): string {
  const bytes = new Uint8Array(length)
  crypto.getRandomValues(bytes)
  let out = ''
  for (const b of bytes) out += (b % 36).toString(36)
  return out
}

export function eventKey(day: string, n: string, dims: Dims, nowMs: number, random = randomBase36(8)): string {
  return `${EVENT_PREFIX}${day}/${n}/${encodeDims(dims)}/${nowMs.toString(36)}-${random}`
}

export interface ParsedEventKey {
  day: string
  n: string
  dims: string
}

const ID_RE = /^[0-9a-z]+-[0-9a-z]+$/

/**
 * Splits an event key; returns null for anything that is not an event key. Accepts the key as
 * written (`p=%2Fapp%2F`) and as it would come back if the storage decoded the `%2F` of a page
 * into `/` (more than five segments); both give the same, encoded, `dims`.
 */
export function parseEventKey(key: string): ParsedEventKey | null {
  const parts = key.split('/')
  if (parts.length < 5 || parts[0] !== 'e') return null
  const [, day, n] = parts
  const id = parts[parts.length - 1]
  let dims = parts.slice(3, -1).join('/')
  if (!isDay(day) || !n || !ID_RE.test(id)) return null
  if (dims !== EMPTY_DIMS && !dims.includes('=')) return null
  if (parts.length > 5) dims = encodeDims(parseDims(dims))
  return { day, n, dims }
}

export function dayPrefix(day: string): string {
  return `${EVENT_PREFIX}${day}/`
}

export function rollupKey(day: string): string {
  return `${ROLLUP_PREFIX}${day}`
}

/** The day named by a directory or key such as `e/2026-11-01`, `e/2026-11-01/` or `r/2026-11-01`. */
export function dayFromPath(path: string): string | null {
  const day = path.replace(/\/+$/, '').split('/')[1] ?? ''
  return isDay(day) ? day : null
}
