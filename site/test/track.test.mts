// Runs the browser beacon (src/assets/track.js) against a fake window and checks that
// every payload it builds is accepted by the server allowlist.
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { describe, it } from 'node:test'
import { validateEvent } from '../netlify/lib/validate.mts'

const SITE = 'https://pdf-edit-mac.netlify.app'
const MAC = 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.1 Safari/605.1.15'
const TRACK_URL = new URL('../src/assets/track.js', import.meta.url)

class FakeElement {
  dataset: Record<string, string>
  constructor(dataset: Record<string, string>) {
    this.dataset = dataset
  }
  closest(selector: string) {
    return selector === '[data-track="gh"]' && this.dataset.track === 'gh' ? this : null
  }
}

interface Setup {
  url?: string
  referrer?: string
  ua?: string
  maxTouchPoints?: number
  mobile?: boolean
  gpc?: boolean
  dnt?: string
  webdriver?: boolean
  attrs?: Record<string, string>
  page?: string
  beaconOk?: boolean
  /** localStorage contents; `null`: storage throws (blocked). */
  storage?: Record<string, string> | null
  links?: { href: string; search: string }[]
  /** The page is inside a frame (the app in /en-ligne/). */
  framed?: boolean
}

let run = 0
async function load(setup: Setup = {}) {
  const sent: { via: string; url: string; body: unknown }[] = []
  const listeners: Record<string, ((e: unknown) => void)[]> = {}
  const attrs = setup.attrs ?? {}
  const fakeNavigator = {
    userAgent: setup.ua ?? MAC,
    maxTouchPoints: setup.maxTouchPoints ?? 0,
    globalPrivacyControl: setup.gpc,
    doNotTrack: setup.dnt,
    webdriver: setup.webdriver ?? false,
    sendBeacon(url: string, blob: Blob) {
      if (setup.beaconOk === false) return false
      sent.push({ via: 'beacon', url, body: blob })
      return true
    },
  }
  const define = (name: string, value: unknown) => Object.defineProperty(globalThis, name, { value, configurable: true, writable: true })
  define('navigator', fakeNavigator)
  define('location', new URL(setup.url ?? `${SITE}/`))
  define('self', globalThis)
  define('top', setup.framed ? {} : globalThis)
  define('matchMedia', () => ({ matches: setup.mobile ?? false }))
  define('Element', FakeElement)
  define(
    'localStorage',
    setup.storage === null
      ? new Proxy({}, { get: () => { throw new Error('SecurityError') } })
      : { ...(setup.storage ?? {}) },
  )
  define('fetch', async (url: string, init: RequestInit) => {
    sent.push({ via: 'fetch', url, body: init.body })
    return new Response(null, { status: 204 })
  })
  define('document', {
    referrer: setup.referrer ?? '',
    documentElement: {
      dataset: setup.page ? { page: setup.page } : {},
      hasAttribute: (name: string) => name in attrs,
    },
    addEventListener: (type: string, fn: (e: unknown) => void) => (listeners[type] ??= []).push(fn),
    querySelectorAll: (selector: string) => (selector === 'a[href^="/telecharger/"]' ? setup.links ?? [] : []),
  })
  const mod = await import(`${TRACK_URL.href}?run=${++run}`)
  // Let the beacon calls settle, then decode the bodies.
  await new Promise((r) => setTimeout(r, 0))
  const decode = async () =>
    Promise.all(sent.map(async (s) => ({ ...s, body: JSON.parse(s.body instanceof Blob ? await s.body.text() : String(s.body)) })))
  return { mod, sent, decode, listeners }
}

async function pageview(setup: Setup) {
  const { decode } = await load(setup)
  const events = await decode()
  return events.map((e) => e.body)
}

describe('track.js page view', () => {
  it('a visit from Google on the landing page', async () => {
    const [pv] = await pageview({ referrer: 'https://www.google.fr/', url: `${SITE}/` })
    assert.deepEqual(pv, { n: 'pv', p: '/', e: 1, r: 'google', d: 'desktop', os: 'mac' })
  })

  it('an internal navigation is not an entry and has no source', async () => {
    const [pv] = await pageview({ referrer: `${SITE}/`, url: `${SITE}/app/` })
    assert.deepEqual(pv, { n: 'pv', p: '/app/', e: 0, d: 'desktop', os: 'mac' })
  })

  it('direct visit with a campaign, on a phone', async () => {
    const [pv] = await pageview({
      url: `${SITE}/?utm_source=Lettre%20Info%20%C3%A9t%C3%A9`,
      ua: 'Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 Chrome/130.0.0.0 Mobile Safari/537.36',
      mobile: true,
    })
    assert.deepEqual(pv, { n: 'pv', p: '/', e: 1, r: 'direct', u: 'lettre-info-t', d: 'mobile', os: 'android' })
  })

  it('normalises paths', async () => {
    const cases: [string, string][] = [
      ['/index.html', '/'],
      ['/app', '/app/'],
      ['/app/index.html', '/app/'],
      ['/app/drop.html', '/app/'],
      ['/app/edit.html', '/app/edit.html'],
      ['/app/edit', '/app/edit.html'],
      ['/mentions-legales', '/mentions-legales/'],
      ['/en-ligne/', '/en-ligne/'],
      ['/en-ligne', '/en-ligne/'],
      ['/en-ligne/index.html', '/en-ligne/'],
      ['/questions/', '/questions/'],
      ['/questions', '/questions/'],
      ['/questions/index.html', '/questions/'],
      ['/en-ligne/questions/', 'other'],
      ['/stats/', 'other'],
      ['/wp-login.php', 'other'],
    ]
    for (const [path, p] of cases) {
      const [pv] = await pageview({ url: `${SITE}${path}` })
      assert.equal((pv as { p: string }).p, p, path)
      assert.ok(validateEvent(pv), `server accepts ${path}`)
    }
    const [notFound] = await pageview({ url: `${SITE}/nope`, page: '404' })
    assert.equal((notFound as { p: string }).p, '/404')
  })

  it('groups referrers', async () => {
    const cases: [string, string][] = [
      ['https://mail.google.com/mail/u/0/', 'gmail'],
      ['https://www.google.com/search?q=x', 'google'],
      ['https://www.google.co.uk/', 'google'],
      ['https://fr.search.yahoo.com/', 'yahoo'],
      ['https://www.bing.com/', 'bing'],
      ['https://duckduckgo.com/', 'duckduckgo'],
      ['https://www.qwant.com/', 'qwant'],
      ['https://www.ecosia.org/', 'ecosia'],
      ['https://search.brave.com/', 'brave'],
      ['https://github.com/Makio64', 'github'],
      ['https://t.co/abc', 'x'],
      ['https://x.com/', 'x'],
      ['https://m.facebook.com/', 'facebook'],
      ['https://l.instagram.com/', 'instagram'],
      ['https://lnkd.in/xyz', 'linkedin'],
      ['https://www.linkedin.com/feed/', 'linkedin'],
      ['https://old.reddit.com/', 'reddit'],
      ['https://news.ycombinator.com/item?id=1', 'hackernews'],
      ['https://m.youtube.com/', 'youtube'],
      ['https://youtu.be/x', 'youtube'],
      ['https://outlook.live.com/mail/', 'outlook'],
      ['https://outlook.office.com/mail/', 'outlook'],
      ['https://www.Blog.Example.org/article?id=1', 'blog.example.org'],
      ['https://notgithub.com/', 'notgithub.com'],
      ['http://203.0.113.7/', 'other'],
      ['http://203.0.113.7:8080/nas/', 'other'],
      ['http://[2001:db8::1]/', 'other'],
      ['http://localhost:3000/', 'other'],
      ['http://nas.localhost/', 'other'],
      [`https://${'a'.repeat(70)}.fr/`, 'a'.repeat(60)],
    ]
    for (const [referrer, r] of cases) {
      const [pv] = await pageview({ referrer })
      assert.equal((pv as { r: string }).r, r, referrer)
      assert.ok(validateEvent(pv), `server accepts ${referrer}`)
    }
  })

  it('reads an iPad that asks for the desktop site as ios', async () => {
    const [pv] = await pageview({ maxTouchPoints: 5 })
    assert.equal((pv as { os: string }).os, 'ios')
  })

  it('falls back to fetch when sendBeacon refuses', async () => {
    const { decode } = await load({ beaconOk: false })
    const [event] = await decode()
    assert.equal(event.via, 'fetch')
    assert.equal(event.url, '/api/e')
  })

  it('sends nothing with GPC, Do Not Track, automation, the opt-out flag, or on the stats page', async () => {
    for (const setup of [
      { gpc: true },
      { dnt: '1' },
      { webdriver: true },
      { storage: { 'pdfedit.optout': '1' } },
      { attrs: { 'data-no-pageview': '' } },
    ]) {
      const { sent } = await load(setup)
      assert.equal(sent.length, 0, JSON.stringify(setup))
    }
  })
})

describe('track.js clicks and track()', () => {
  it('counts GitHub clicks with an allowlisted source', async () => {
    const { listeners, decode, sent } = await load({ attrs: { 'data-no-pageview': '' } })
    const [click] = listeners.click
    click({ type: 'click', button: 0, target: new FakeElement({ track: 'gh', trackSrc: 'footer' }) })
    click({ type: 'click', button: 0, target: new FakeElement({ track: 'gh', trackSrc: 'hero' }) })
    click({ type: 'click', button: 0, target: new FakeElement({}) })
    listeners.auxclick[0]({ type: 'auxclick', button: 2, target: new FakeElement({ track: 'gh', trackSrc: 'faq' }) })
    listeners.auxclick[0]({ type: 'auxclick', button: 1, target: new FakeElement({ track: 'gh', trackSrc: 'faq' }) })
    assert.equal(sent.length, 3)
    const bodies = (await decode()).map((e) => e.body)
    assert.deepEqual(bodies, [
      { n: 'gh', src: 'footer' },
      { n: 'gh', src: 'other' },
      { n: 'gh', src: 'faq' },
    ])
    for (const body of bodies) assert.ok(validateEvent(body))
  })

  it('exports track() for the in-browser app', async () => {
    const { mod, decode } = await load({ attrs: { 'data-no-pageview': '' } })
    mod.track('conv', { f: 'docx', ok: 1, t: 'lt5' })
    const [event] = await decode()
    assert.deepEqual(event.body, { n: 'conv', f: 'docx', ok: 1, t: 'lt5' })
    assert.ok(validateEvent(event.body))
  })

  it('with the opt-out flag, marks the download links so the server skips them', async () => {
    const links = [{ href: '/telecharger/mac?from=hero', search: '?from=hero' }]
    const { sent, mod } = await load({ storage: { 'pdfedit.optout': '1' }, links })
    assert.equal(links[0].search, '?from=hero&nc=1')
    mod.track('conv', { f: 'docx', ok: 1 })
    assert.equal(sent.length, 0)
    const untouched = [{ href: '/telecharger/mac?from=hero', search: '?from=hero' }]
    await load({ links: untouched })
    assert.equal(untouched[0].search, '?from=hero')
  })

  it('sends no page view inside a frame, but still counts the app there', async () => {
    const { sent, mod, decode } = await load({ framed: true, url: `${SITE}/app/` })
    assert.equal(sent.length, 0)
    mod.track('eopen', { ok: 1 })
    const [event] = await decode()
    assert.deepEqual(event.body, { n: 'eopen', ok: 1 })
    const { sent: editor } = await load({ framed: true, url: `${SITE}/app/edit.html?session=x` })
    assert.equal(editor.length, 0)
  })

  it('counts the app pages opened on their own', async () => {
    const [pv] = await pageview({ url: `${SITE}/app/edit.html?session=x`, referrer: `${SITE}/en-ligne/` })
    assert.deepEqual(pv, { n: 'pv', p: '/app/edit.html', e: 0, d: 'desktop', os: 'mac' })
  })

  it('a visit sent on from an old /app/ link is an entry of /en-ligne/', async () => {
    // The redirect of /app/ (scripts/build-web.sh) sets no-referrer: the page arrives without one.
    const [pv] = await pageview({ url: `${SITE}/en-ligne/`, referrer: '' })
    assert.deepEqual(pv, { n: 'pv', p: '/en-ligne/', e: 1, r: 'direct', d: 'desktop', os: 'mac' })
  })

  it('counts normally when storage is blocked', async () => {
    const { sent } = await load({ storage: null })
    assert.equal(sent.length, 1)
  })

  it('stays under 3 KB', () => {
    assert.ok(readFileSync(TRACK_URL).byteLength < 3072)
  })
})
