// Request handlers behind the three functions. They take their dependencies
// (store, waitUntil, clock, env, release) as arguments so tests can run them in Node.
// Nothing personal is logged: no IP, no user agent, no body.

import { buildStats, parseRange } from './aggregate.mts'
import { githubDownloads } from './github.mts'
import {
  asksNotToBeTracked,
  bearerToken,
  constantTimeEqual,
  empty,
  json,
  mediaType,
  originAllowed,
  readBodyCapped,
  MAX_BODY_BYTES,
} from './http.mts'
import { appendEvent, type StatsStore } from './store.mts'
import { DL_SOURCES, EVENTS, isBot, osFromUA, validateEvent, type ValidEvent } from './validate.mts'

export interface Release {
  version: string
  file: string
  url: string
  bytes: number
  sha256: string
}

interface BaseDeps {
  /** Called lazily, only when the store is actually needed. */
  store: () => StatsStore
  now?: () => number
}
export interface EventDeps extends BaseDeps {
  waitUntil: (promise: Promise<unknown>) => void
}
export interface DownloadDeps extends EventDeps {
  release: Release
}
export interface StatsDeps extends BaseDeps {
  env: (name: string) => string | undefined
  fetch?: (url: string, init?: RequestInit) => Promise<Response>
}

/** Shorter STATS_KEY values count as not configured: the key is the dashboard's only protection. */
export const MIN_STATS_KEY_LENGTH = 24

/**
 * Stores one event in the background. Never throws: counting must not break the 204 of /api/e
 * nor the redirect of /telecharger/mac, even when the Blobs context is missing (getStore throws).
 */
function record(deps: EventDeps, event: ValidEvent): void {
  const nowMs = (deps.now ?? Date.now)()
  const write = Promise.resolve()
    .then(() => appendEvent(deps.store(), event, nowMs))
    .catch(() => {
      console.error(`analytics: could not store a "${event.n}" event`)
    })
  try {
    deps.waitUntil(write)
  } catch {
    // No waitUntil: the write still runs, it may just be cut short.
  }
}

/** Parses and validates a browser event; null means "drop it silently". */
export async function readEvent(req: Request): Promise<ValidEvent | null> {
  if (isBot(req.headers.get('user-agent'))) return null
  // track.js already sends nothing then; this covers an old cached copy or another client.
  if (asksNotToBeTracked(req)) return null
  if (!originAllowed(req)) return null
  const type = mediaType(req)
  if (type !== 'text/plain' && type !== 'application/json') return null
  const text = await readBodyCapped(req, MAX_BODY_BYTES)
  if (!text) return null
  let data: unknown
  try {
    data = JSON.parse(text)
  } catch {
    return null
  }
  return validateEvent(data, { serverSide: false })
}

/** POST /api/e: always 204 with an empty body, whether the event was kept or not. */
export async function handleEvent(req: Request, deps: EventDeps): Promise<Response> {
  if (req.method !== 'POST') return empty(405, { allow: 'POST' })
  let event: ValidEvent | null = null
  try {
    event = await readEvent(req)
  } catch {
    event = null
  }
  if (event) record(deps, event)
  return empty(204)
}

function isPrefetch(req: Request): boolean {
  const purpose = `${req.headers.get('sec-purpose') ?? ''} ${req.headers.get('purpose') ?? ''} ${req.headers.get('x-moz') ?? ''}`
  return /prefetch|prerender/i.test(purpose)
}

const VERSION_RE = EVENTS.dl.dims.v.re as RegExp

/**
 * GET|HEAD /telecharger/mac?from=<src>: counts the download, then 302 to the DMG. Not counted: HEAD,
 * bots, prefetches, browsers sending GPC or Do Not Track, and links marked `nc=1` (the « Ne plus
 * être compté » choice of the privacy page, which track.js adds to the download links).
 */
export async function handleDownload(req: Request, deps: DownloadDeps): Promise<Response> {
  if (req.method !== 'GET' && req.method !== 'HEAD') return empty(405, { allow: 'GET, HEAD' })
  const url = new URL(req.url)
  if (!deps.release?.url) return empty(404)
  const location = new URL(deps.release.url, url).href

  const ua = req.headers.get('user-agent')
  const counted = req.method === 'GET' && !isBot(ua) && !isPrefetch(req) && !asksNotToBeTracked(req) && url.searchParams.get('nc') !== '1'
  if (counted) {
    const from = url.searchParams.get('from') ?? ''
    const src = (DL_SOURCES as readonly string[]).includes(from) ? from : 'other'
    const version = VERSION_RE.test(deps.release.version) ? deps.release.version : null
    const event = validateEvent({ n: 'dl', v: version, src, os: osFromUA(ua) }, { serverSide: true })
    if (event) record(deps, event)
  }
  return empty(302, { location })
}

/** GET /api/stats?days=7|30|90|365|all with `Authorization: Bearer <STATS_KEY>`. */
export async function handleStats(req: Request, deps: StatsDeps): Promise<Response> {
  if (req.method !== 'GET') return json(405, { error: 'method not allowed' }, { allow: 'GET' })
  const secret = deps.env('STATS_KEY')
  if (!secret || secret.length < MIN_STATS_KEY_LENGTH) return json(503, { error: 'not configured' })
  const token = bearerToken(req)
  if (!token || !constantTimeEqual(token, secret)) {
    return json(401, { error: 'unauthorized' }, { 'www-authenticate': 'Bearer' })
  }

  try {
    const nowMs = (deps.now ?? Date.now)()
    const store = deps.store()
    const range = parseRange(new URL(req.url).searchParams.get('days'))
    const [stats, github] = await Promise.all([
      buildStats(store, { range, nowMs }),
      githubDownloads(store, deps.env('GITHUB_REPO'), { fetch: deps.fetch, nowMs }),
    ])
    return json(200, { ...stats, github })
  } catch {
    console.error('stats: could not build the statistics')
    return json(500, { error: 'server error' })
  }
}
