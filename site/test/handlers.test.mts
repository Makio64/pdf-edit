import assert from 'node:assert/strict'
import { describe, it } from 'node:test'
import { CACHE_TTL_MS, githubDownloads } from '../netlify/lib/github.mts'
import { handleDownload, handleEvent, handleStats, MIN_STATS_KEY_LENGTH, type Release } from '../netlify/lib/handlers.mts'
import { constantTimeEqual, originAllowed } from '../netlify/lib/http.mts'
import { GITHUB_CACHE_KEY, parseDims, parseEventKey } from '../netlify/lib/keys.mts'
import { memoryStore, type MemoryStore } from '../netlify/lib/memory-store.mts'

const SITE = 'https://pdf-edit-mac.netlify.app'
const SAFARI = 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.1 Safari/605.1.15'
const WINDOWS = 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36'
const NOW = Date.UTC(2026, 10, 1, 12)

function eventDeps(store: MemoryStore) {
  const pending: Promise<unknown>[] = []
  let opened = 0
  return {
    pending,
    get opened() {
      return opened
    },
    deps: {
      store: () => {
        opened++
        return store
      },
      waitUntil: (p: Promise<unknown>) => void pending.push(p),
      now: () => NOW,
    },
  }
}

function beacon(body: string, headers: Record<string, string> = {}, method = 'POST'): Request {
  return new Request(`${SITE}/api/e`, {
    method,
    body: method === 'POST' ? body : undefined,
    headers: { origin: SITE, 'user-agent': SAFARI, 'content-type': 'text/plain;charset=UTF-8', ...headers },
  })
}

async function send(req: Request) {
  const store = memoryStore()
  const ctx = eventDeps(store)
  const res = await handleEvent(req, ctx.deps)
  await Promise.all(ctx.pending)
  const keys = [...store.data.keys()]
  return { res, keys, store, opened: ctx.opened }
}

describe('POST /api/e', () => {
  it('stores a valid page view and answers 204 with an empty body', async () => {
    const { res, keys } = await send(beacon(JSON.stringify({ n: 'pv', p: '/', e: 1, r: 'google', d: 'desktop', os: 'mac' })))
    assert.equal(res.status, 204)
    assert.equal(await res.text(), '')
    assert.equal(keys.length, 1)
    const parsed = parseEventKey(keys[0])!
    assert.deepEqual(parsed.day, '2026-11-01')
    assert.equal(parsed.n, 'pv')
    assert.deepEqual(parseDims(parsed.dims), { p: '/', e: '1', r: 'google', d: 'desktop', os: 'mac' })
  })

  it('accepts application/json too', async () => {
    const { keys } = await send(beacon('{"n":"gh","src":"footer"}', { 'content-type': 'application/json' }))
    assert.equal(keys.length, 1)
  })

  it('accepts a local development origin on any port', async () => {
    const { keys } = await send(beacon('{"n":"pw","w":"conv"}', { origin: 'http://localhost:8895' }))
    assert.equal(keys.length, 1)
    const { keys: keys2 } = await send(beacon('{"n":"pw","w":"conv"}', { origin: 'http://127.0.0.1:7878' }))
    assert.equal(keys2.length, 1)
  })

  it('falls back to Referer when Origin is absent', async () => {
    const req = new Request(`${SITE}/api/e`, {
      method: 'POST',
      body: '{"n":"pv","p":"/"}',
      headers: { referer: `${SITE}/app/`, 'user-agent': SAFARI, 'content-type': 'text/plain' },
    })
    assert.equal((await send(req)).keys.length, 1)
  })

  const dropped: [string, Request][] = [
    ['dl (server-side only)', beacon(JSON.stringify({ n: 'dl', v: '1.1.1', src: 'hero', os: 'mac' }))],
    ['unknown event', beacon('{"n":"signup"}')],
    ['unknown dimension', beacon('{"n":"pv","p":"/","name":"Jean"}')],
    ['invalid JSON', beacon('{"n":"pv",')],
    ['empty body', beacon('')],
    ['oversize body', beacon(JSON.stringify({ n: 'pv', p: '/', pad: 'x'.repeat(1100) }))],
    ['oversize body without the extra key', beacon(`{"n":"pv","p":"/"}${' '.repeat(1100)}`)],
    ['foreign origin', beacon('{"n":"pv","p":"/"}', { origin: 'https://evil.example' })],
    ['look-alike origin', beacon('{"n":"pv","p":"/"}', { origin: 'https://pdf-edit-mac.netlify.app.evil.example' })],
    ['origin null without referer', beacon('{"n":"pv","p":"/"}', { origin: 'null' })],
    [
      'no origin and no referer',
      new Request(`${SITE}/api/e`, { method: 'POST', body: '{"n":"pv","p":"/"}', headers: { 'user-agent': SAFARI, 'content-type': 'text/plain' } }),
    ],
    ['bot user agent', beacon('{"n":"pv","p":"/"}', { 'user-agent': 'Mozilla/5.0 (compatible; Googlebot/2.1)' })],
    ['curl', beacon('{"n":"pv","p":"/"}', { 'user-agent': 'curl/8.7.1' })],
    ['headless browser', beacon('{"n":"pv","p":"/"}', { 'user-agent': SAFARI.replace('Safari', 'HeadlessChrome') })],
    ['form content type', beacon('{"n":"pv","p":"/"}', { 'content-type': 'application/x-www-form-urlencoded' })],
    ['Global Privacy Control', beacon('{"n":"pv","p":"/"}', { 'sec-gpc': '1' })],
    ['Do Not Track', beacon('{"n":"pv","p":"/"}', { dnt: '1' })],
  ]
  for (const [label, req] of dropped) {
    it(`drops ${label} silently with 204`, async () => {
      const { res, keys } = await send(req)
      assert.equal(res.status, 204)
      assert.equal(await res.text(), '')
      assert.equal(keys.length, 0)
    })
  }

  it('does not open the store for dropped events', async () => {
    const { opened } = await send(beacon('{"n":"nope"}'))
    assert.equal(opened, 0)
  })

  it('answers 405 to other methods', async () => {
    const { res, keys } = await send(beacon('', {}, 'GET'))
    assert.equal(res.status, 405)
    assert.equal(keys.length, 0)
  })

  it('still answers 204 when the store cannot even be opened, or waitUntil is missing', async () => {
    const original = console.error
    const errors: string[] = []
    console.error = (msg: string) => void errors.push(msg)
    try {
      const pending: Promise<unknown>[] = []
      const throwing = {
        store: () => {
          throw new Error('MissingBlobsEnvironmentError')
        },
        waitUntil: (p: Promise<unknown>) => void pending.push(p),
        now: () => NOW,
      }
      const res = await handleEvent(beacon('{"n":"pv","p":"/"}'), throwing)
      assert.equal(res.status, 204)
      await Promise.all(pending) // swallowed, never rejects
      const noWaitUntil = {
        store: () => memoryStore(),
        waitUntil: () => {
          throw new TypeError('context.waitUntil is not a function')
        },
      }
      assert.equal((await handleEvent(beacon('{"n":"pv","p":"/"}'), noWaitUntil)).status, 204)
      assert.deepEqual(errors, ['analytics: could not store a "pv" event'])
    } finally {
      console.error = original
    }
  })

  it('still answers 204 when the store fails', async () => {
    const store = memoryStore()
    store.set = async () => {
      throw new Error('blobs down')
    }
    const ctx = eventDeps(store)
    const original = console.error
    console.error = () => {}
    try {
      const res = await handleEvent(beacon('{"n":"pv","p":"/"}'), ctx.deps)
      assert.equal(res.status, 204)
      await Promise.all(ctx.pending) // swallowed, never rejects
    } finally {
      console.error = original
    }
  })
})

describe('originAllowed', () => {
  it('compares the host, port included', () => {
    const req = (origin: string) => new Request('https://example.org/api/e', { method: 'POST', headers: { origin } })
    assert.equal(originAllowed(req('https://example.org')), true)
    assert.equal(originAllowed(req('https://EXAMPLE.org')), true)
    assert.equal(originAllowed(req('https://example.org:8443')), false)
    assert.equal(originAllowed(req('https://www.example.org')), false)
    assert.equal(originAllowed(req('ftp://example.org')), false)
    assert.equal(originAllowed(req('not a url')), false)
    assert.equal(originAllowed(req('http://localhost')), true)
  })
})

const RELEASE: Release = {
  version: '1.1.1',
  file: 'PDF-Edit-1.1.1.dmg',
  url: '/downloads/PDF-Edit-1.1.1.dmg',
  bytes: 7340032,
  sha256: 'abc',
}

async function download(path: string, init: RequestInit & { headers?: Record<string, string> } = {}, release = RELEASE) {
  const store = memoryStore()
  const ctx = eventDeps(store)
  const req = new Request(`${SITE}${path}`, { ...init, headers: { 'user-agent': SAFARI, ...init.headers } })
  const res = await handleDownload(req, { ...ctx.deps, release })
  await Promise.all(ctx.pending)
  return { res, keys: [...store.data.keys()] }
}

describe('GET /telecharger/mac', () => {
  it('counts the download and redirects to the DMG', async () => {
    const { res, keys } = await download('/telecharger/mac?from=hero')
    assert.equal(res.status, 302)
    assert.equal(res.headers.get('location'), `${SITE}/downloads/PDF-Edit-1.1.1.dmg`)
    assert.equal(res.headers.get('cache-control'), 'no-store')
    assert.equal(keys.length, 1)
    const parsed = parseEventKey(keys[0])!
    assert.equal(parsed.n, 'dl')
    assert.deepEqual(parseDims(parsed.dims), { v: '1.1.1', src: 'hero', os: 'mac' })
  })

  it('maps an unknown or missing source to other and reads the OS', async () => {
    const { keys } = await download('/telecharger/mac?from=<script>', { headers: { 'user-agent': WINDOWS } })
    assert.deepEqual(parseDims(parseEventKey(keys[0])!.dims), { v: '1.1.1', src: 'other', os: 'windows' })
    const { keys: noFrom } = await download('/telecharger/mac')
    assert.deepEqual(parseDims(parseEventKey(noFrom[0])!.dims), { v: '1.1.1', src: 'other', os: 'mac' })
  })

  it('redirects to a GitHub release URL as is', async () => {
    const url = 'https://github.com/Makio64/pdf-edit/releases/download/v1.1.1/PDF-Edit-1.1.1.dmg'
    const { res } = await download('/telecharger/mac?from=app', {}, { ...RELEASE, url })
    assert.equal(res.headers.get('location'), url)
  })

  it('does not count HEAD, bots or prefetches, but still redirects', async () => {
    for (const init of [
      { method: 'HEAD' },
      { headers: { 'user-agent': 'Wget/1.21' } },
      { headers: { 'user-agent': 'Mozilla/5.0 (compatible; bingbot/2.0)' } },
      { headers: { 'sec-purpose': 'prefetch;prerender' } },
    ]) {
      const { res, keys } = await download('/telecharger/mac?from=hero', init)
      assert.equal(res.status, 302, JSON.stringify(init))
      assert.equal(keys.length, 0, JSON.stringify(init))
    }
  })

  it('does not count with GPC, Do Not Track or nc=1, but still redirects', async () => {
    for (const [path, headers] of [
      ['/telecharger/mac?from=hero', { 'sec-gpc': '1' }],
      ['/telecharger/mac?from=hero', { dnt: '1' }],
      ['/telecharger/mac?from=hero&nc=1', {}],
    ] as [string, Record<string, string>][]) {
      const { res, keys } = await download(path, { headers })
      assert.equal(res.status, 302, path + JSON.stringify(headers))
      assert.equal(res.headers.get('location'), `${SITE}/downloads/PDF-Edit-1.1.1.dmg`)
      assert.equal(keys.length, 0, path + JSON.stringify(headers))
    }
    // DNT: 0 is not a request to be left alone.
    assert.equal((await download('/telecharger/mac?from=hero', { headers: { dnt: '0' } })).keys.length, 1)
  })

  it('still redirects when the store cannot be opened', async () => {
    const original = console.error
    console.error = () => {}
    try {
      const pending: Promise<unknown>[] = []
      const req = new Request(`${SITE}/telecharger/mac?from=hero`, { headers: { 'user-agent': SAFARI } })
      const res = await handleDownload(req, {
        store: () => {
          throw new Error('MissingBlobsEnvironmentError')
        },
        waitUntil: (p) => void pending.push(p),
        release: RELEASE,
      })
      assert.equal(res.status, 302)
      assert.equal(res.headers.get('location'), `${SITE}/downloads/PDF-Edit-1.1.1.dmg`)
      await Promise.all(pending)
    } finally {
      console.error = original
    }
  })

  it('answers 405 to POST', async () => {
    const { res, keys } = await download('/telecharger/mac', { method: 'POST' })
    assert.equal(res.status, 405)
    assert.equal(keys.length, 0)
  })

  it('drops an invalid version but keeps the count', async () => {
    const { keys } = await download('/telecharger/mac?from=faq', {}, { ...RELEASE, version: 'beta' })
    assert.deepEqual(parseDims(parseEventKey(keys[0])!.dims), { src: 'faq', os: 'mac' })
  })
})

describe('GET /api/stats', () => {
  const KEY = 'k3y-with-enough-entropy-0123456789'
  function stats(headers: Record<string, string> = {}, { env = { STATS_KEY: KEY } as Record<string, string>, path = '/api/stats', store = memoryStore(), fetch = undefined as any } = {}) {
    return handleStats(new Request(`${SITE}${path}`, { headers }), {
      store: () => store,
      env: (name) => env[name],
      now: () => NOW,
      fetch: fetch ?? (async () => new Response('[]', { status: 200 })),
    })
  }

  it('503 when STATS_KEY is not set', async () => {
    const res = await stats({ authorization: `Bearer ${KEY}` }, { env: {} })
    assert.equal(res.status, 503)
    assert.deepEqual(await res.json(), { error: 'not configured' })
  })

  it('503 when STATS_KEY is too short to be a real secret', async () => {
    const short = 'x'.repeat(MIN_STATS_KEY_LENGTH - 1)
    const res = await stats({ authorization: `Bearer ${short}` }, { env: { STATS_KEY: short } })
    assert.equal(res.status, 503)
    assert.deepEqual(await res.json(), { error: 'not configured' })
    const enough = 'x'.repeat(MIN_STATS_KEY_LENGTH)
    assert.equal((await stats({ authorization: `Bearer ${enough}` }, { env: { STATS_KEY: enough } })).status, 200)
  })

  it('401 without or with a wrong key', async () => {
    for (const authorization of [undefined, '', 'Bearer', `Bearer ${KEY}x`, `Basic ${KEY}`, KEY, `Bearer ${KEY.slice(0, -1)}`]) {
      const res = await stats(authorization === undefined ? {} : { authorization })
      assert.equal(res.status, 401, String(authorization))
      assert.deepEqual(await res.json(), { error: 'unauthorized' })
      assert.equal(res.headers.get('cache-control'), 'no-store')
    }
  })

  it('200 with the right key, no-store, default 30 days', async () => {
    const store = memoryStore()
    const ctx = eventDeps(store)
    await handleEvent(beacon('{"n":"pv","p":"/","e":1,"r":"direct"}'), ctx.deps)
    await Promise.all(ctx.pending)
    const res = await stats({ authorization: `Bearer ${KEY}` }, { store })
    assert.equal(res.status, 200)
    assert.equal(res.headers.get('cache-control'), 'no-store')
    assert.match(res.headers.get('content-type')!, /^application\/json/)
    const body = await res.json()
    assert.equal(body.range.days, 30)
    assert.equal(body.series.length, 30)
    assert.equal(body.totals.visits, 1)
    assert.equal(body.allTime.pv, 1)
    assert.equal(body.github, null)
    assert.deepEqual(Object.keys(body).sort(), ['allTime', 'breakdowns', 'generatedAt', 'github', 'range', 'series', 'totals'])
  })

  it('honours ?days=', async () => {
    for (const [days, expected] of [['7', 7], ['90', 90], ['365', 365], ['all', 1], ['12', 30]] as const) {
      const res = await stats({ authorization: `Bearer ${KEY}` }, { path: `/api/stats?days=${days}` })
      assert.equal((await res.json()).range.days, expected, days)
    }
  })

  it('fills github when GITHUB_REPO is set', async () => {
    const fetch = async () =>
      Response.json([
        { assets: [{ name: 'PDF-Edit-1.1.1.dmg', download_count: 12 }, { name: 'checksums.txt', download_count: 99 }] },
        { assets: [{ name: 'Modifier-mes-PDF-1.1.0.dmg', download_count: 30 }] },
      ])
    const res = await stats({ authorization: `Bearer ${KEY}` }, { env: { STATS_KEY: KEY, GITHUB_REPO: 'Makio64/pdf-edit' }, fetch })
    const { github } = await res.json()
    assert.equal(github.total, 42)
    assert.deepEqual(github.assets, [
      { name: 'Modifier-mes-PDF-1.1.0.dmg', downloads: 30 },
      { name: 'PDF-Edit-1.1.1.dmg', downloads: 12 },
    ])
  })

  it('500 without details when the store breaks', async () => {
    const store = memoryStore()
    store.list = async () => {
      throw new Error('boom')
    }
    const original = console.error
    console.error = () => {}
    try {
      const res = await stats({ authorization: `Bearer ${KEY}` }, { store })
      assert.equal(res.status, 500)
      assert.deepEqual(await res.json(), { error: 'server error' })
    } finally {
      console.error = original
    }
  })
})

describe('constantTimeEqual', () => {
  it('compares secrets of any length', () => {
    assert.equal(constantTimeEqual('abc', 'abc'), true)
    assert.equal(constantTimeEqual('abc', 'abd'), false)
    assert.equal(constantTimeEqual('abc', 'abcd'), false)
    assert.equal(constantTimeEqual('', 'a'), false)
    assert.equal(constantTimeEqual('clé-é', 'clé-é'), true)
  })
})

describe('githubDownloads', () => {
  const repo = 'Makio64/pdf-edit'
  it('caches for an hour', async () => {
    const store = memoryStore()
    let calls = 0
    const fetch = async (url: string) => {
      calls++
      assert.equal(url, 'https://api.github.com/repos/Makio64/pdf-edit/releases?per_page=100')
      return Response.json([{ assets: [{ name: 'a.dmg', download_count: calls }] }])
    }
    const first = await githubDownloads(store, repo, { fetch, nowMs: NOW })
    assert.equal(first!.total, 1)
    assert.ok(store.data.has(GITHUB_CACHE_KEY))
    const cached = await githubDownloads(store, repo, { fetch, nowMs: NOW + CACHE_TTL_MS - 1 })
    assert.equal(cached!.total, 1)
    assert.equal(calls, 1)
    const fresh = await githubDownloads(store, repo, { fetch, nowMs: NOW + CACHE_TTL_MS + 1 })
    assert.equal(fresh!.total, 2)
    assert.equal(calls, 2)
  })

  it('gives null on any error', async () => {
    const failures = [
      async () => new Response('not found', { status: 404 }),
      async () => new Response('{bad json', { status: 200 }),
      async () => Response.json({ message: 'rate limited' }),
      async () => {
        throw new Error('offline')
      },
    ]
    for (const fetch of failures) assert.equal(await githubDownloads(memoryStore(), repo, { fetch, nowMs: NOW }), null)
  })

  it('ignores a missing or malformed repo without fetching', async () => {
    const fetch = async () => assert.fail('must not fetch')
    assert.equal(await githubDownloads(memoryStore(), undefined, { fetch, nowMs: NOW }), null)
    assert.equal(await githubDownloads(memoryStore(), '../../evil', { fetch, nowMs: NOW }), null)
  })
})
