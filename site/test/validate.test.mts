import assert from 'node:assert/strict'
import { describe, it } from 'node:test'
import { isBot, osFromUA, validateEvent } from '../netlify/lib/validate.mts'

describe('validateEvent: accepted', () => {
  const cases: [unknown, { n: string; dims: Record<string, string> }][] = [
    [{ n: 'pv', p: '/' }, { n: 'pv', dims: { p: '/' } }],
    [
      { n: 'pv', p: '/app/', e: 1, r: 'google', u: 'newsletter_nov-2026', d: 'mobile', os: 'ios' },
      { n: 'pv', dims: { p: '/app/', e: '1', r: 'google', u: 'newsletter_nov-2026', d: 'mobile', os: 'ios' } },
    ],
    [{ n: 'pv', p: '/404', e: '0' }, { n: 'pv', dims: { p: '/404', e: '0' } }],
    [{ n: 'pv', p: '/en-ligne/', e: 1, r: 'direct' }, { n: 'pv', dims: { p: '/en-ligne/', e: '1', r: 'direct' } }],
    [{ n: 'pv', p: '/questions/' }, { n: 'pv', dims: { p: '/questions/' } }],
    [{ n: 'pv', p: '/app/edit.html' }, { n: 'pv', dims: { p: '/app/edit.html' } }],
    [{ n: 'pv', p: 'other', r: 'blog.example.fr' }, { n: 'pv', dims: { p: 'other', r: 'blog.example.fr' } }],
    [{ n: 'conv', f: 'pages', ok: 1, t: 'lt5' }, { n: 'conv', dims: { f: 'pages', ok: '1', t: 'lt5' } }],
    [{ n: 'conv', f: 'docx', ok: false, err: 'other' }, { n: 'conv', dims: { f: 'docx', ok: '0', err: 'other' } }],
    [{ n: 'pw', w: 'edit' }, { n: 'pw', dims: { w: 'edit' } }],
    [{ n: 'pw' }, { n: 'pw', dims: {} }],
    [{ n: 'eopen', ok: 1 }, { n: 'eopen', dims: { ok: '1' } }],
    [{ n: 'esave', ok: 0 }, { n: 'esave', dims: { ok: '0' } }],
    [{ n: 'gh', src: 'opensource' }, { n: 'gh', dims: { src: 'opensource' } }],
    [{ n: 'pv', p: '/', r: null, u: undefined }, { n: 'pv', dims: { p: '/' } }],
  ]
  for (const [input, expected] of cases) {
    it(JSON.stringify(input), () => assert.deepEqual(validateEvent(input), expected))
  }
})

describe('validateEvent: rejected', () => {
  const cases: [string, unknown][] = [
    ['not an object', 'pv'],
    ['array', [{ n: 'pv', p: '/' }]],
    ['null', null],
    ['missing name', { p: '/' }],
    ['unknown event', { n: 'click', p: '/' }],
    ['prototype name', { n: 'constructor' }],
    ['__proto__ name', JSON.parse('{"n":"__proto__"}')],
    ['dl from the browser', { n: 'dl', v: '1.1.1', src: 'hero', os: 'mac' }],
    ['pv without page', { n: 'pv', e: 1 }],
    ['page not in allowlist', { n: 'pv', p: '/secret/' }],
    ['page without its trailing slash', { n: 'pv', p: '/en-ligne' }],
    ['page as an index.html', { n: 'pv', p: '/questions/index.html' }],
    ['unknown dimension', { n: 'pv', p: '/', file: 'facture.pdf' }],
    ['__proto__ dimension', JSON.parse('{"n":"pv","p":"/","__proto__":"x"}')],
    ['uppercase referrer', { n: 'pv', p: '/', r: 'Google' }],
    ['referrer with path', { n: 'pv', p: '/', r: 'example.com/page' }],
    ['referrer too long', { n: 'pv', p: '/', r: 'a'.repeat(61) }],
    ['referrer that is an IPv4 address', { n: 'pv', p: '/', r: '203.0.113.7' }],
    ['referrer made of digits only', { n: 'pv', p: '/', r: '1234' }],
    ['utm with space', { n: 'pv', p: '/', u: 'news letter' }],
    ['utm too long', { n: 'pv', p: '/', u: 'a'.repeat(31) }],
    ['e not a bit', { n: 'pv', p: '/', e: 2 }],
    ['e as float', { n: 'pv', p: '/', e: 0.5 }],
    ['device unknown', { n: 'pv', p: '/', d: 'tablet' }],
    ['os unknown', { n: 'pv', p: '/', os: 'beos' }],
    ['object value', { n: 'pv', p: { toString: '/' } }],
    ['conv without ok', { n: 'conv', f: 'pages' }],
    ['conv without format', { n: 'conv', ok: 1 }],
    ['conv bad format', { n: 'conv', f: 'pdf', ok: 1 }],
    ['conv bad duration', { n: 'conv', f: 'docx', ok: 1, t: '3s' }],
    ['conv error text', { n: 'conv', f: 'docx', ok: 0, err: 'Error: /Users/someone/file.pdf' }],
    ['pw bad where', { n: 'pw', w: 'drop' }],
    ['gh bad src', { n: 'gh', src: 'hero' }],
  ]
  for (const [label, input] of cases) {
    it(label, () => assert.equal(validateEvent(input), null))
  }
})

describe('validateEvent: server-side dl', () => {
  it('accepts dl when recorded by the server', () => {
    assert.deepEqual(validateEvent({ n: 'dl', v: '1.1.1', src: 'hero', os: 'mac' }, { serverSide: true }), {
      n: 'dl',
      dims: { v: '1.1.1', src: 'hero', os: 'mac' },
    })
  })
  it('rejects a bad version or source', () => {
    assert.equal(validateEvent({ n: 'dl', v: 'v1', src: 'hero' }, { serverSide: true }), null)
    assert.equal(validateEvent({ n: 'dl', v: '1.1.1', src: 'opensource' }, { serverSide: true }), null)
  })
})

describe('isBot', () => {
  const bots = [
    'Googlebot/2.1 (+http://www.google.com/bot.html)',
    'Mozilla/5.0 (compatible; bingbot/2.0)',
    'Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) HeadlessChrome/120.0 Safari/537.36',
    'curl/8.7.1',
    'Wget/1.21',
    'python-requests/2.31',
    'axios/1.6.0',
    'Go-http-client/2.0',
    'Java/17.0.2',
    'Mozilla/5.0 (Linux; Android 11; moto g power (2022)) Chrome-Lighthouse',
    'Mozilla/5.0 (compatible; Discordbot/2.0; +https://discordapp.com)',
    'Slackbot-LinkExpanding 1.0',
    '',
    '   ',
  ]
  for (const ua of bots) it(`bot: ${JSON.stringify(ua)}`, () => assert.equal(isBot(ua), true))
  it('no user agent at all', () => assert.equal(isBot(null), true))

  const people = [
    'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.1 Safari/605.1.15',
    'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36 Edg/130.0.0.0',
    'Mozilla/5.0 (iPhone; CPU iPhone OS 18_1 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.1 Mobile/15E148 Safari/604.1',
    'Mozilla/5.0 (X11; Linux x86_64; rv:132.0) Gecko/20100101 Firefox/132.0',
  ]
  for (const ua of people) it(`person: ${ua.slice(0, 40)}…`, () => assert.equal(isBot(ua), false))
})

describe('osFromUA', () => {
  const cases: [string, string][] = [
    ['Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 Safari/605.1.15', 'mac'],
    ['Mozilla/5.0 (iPhone; CPU iPhone OS 18_1 like Mac OS X) AppleWebKit/605.1.15 Mobile/15E148', 'ios'],
    ['Mozilla/5.0 (iPad; CPU OS 17_0 like Mac OS X) AppleWebKit/605.1.15', 'ios'],
    ['Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/130.0.0.0', 'windows'],
    ['Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 Chrome/130.0.0.0 Mobile', 'android'],
    ['Mozilla/5.0 (X11; Linux x86_64; rv:132.0) Gecko/20100101 Firefox/132.0', 'linux'],
    ['Mozilla/5.0 (X11; CrOS x86_64 14541.0.0) AppleWebKit/537.36 Chrome/130.0.0.0', 'chromeos'],
    ['SomethingElse/1.0', 'other'],
    ['', 'other'],
  ]
  for (const [ua, os] of cases) it(`${os}: ${ua.slice(0, 40)}`, () => assert.equal(osFromUA(ua), os))
})
