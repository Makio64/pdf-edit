// PDF Edit's cookieless counts (docs/WEB.md §5). Never throws: the app imports it.
const nav = globalThis.navigator || {}
const de = globalThis.document?.documentElement
let opt
try { opt = localStorage['pdfedit.optout'] === '1' } catch {}
const off = opt || nav.globalPrivacyControl === true || nav.doNotTrack === '1' || nav.webdriver

export function track(name, dims) {
  if (off) return
  try {
    const body = JSON.stringify({ n: name, ...dims })
    if (nav.sendBeacon?.('/api/e', body)) return
    fetch('/api/e', { method: 'POST', body, keepalive: true }).catch(() => {})
  } catch {}
}

const SOURCES = 'gmail mail.google.com|outlook outlook.live.com outlook.office.com|bing|duckduckgo|qwant|ecosia ecosia.org|brave|github|x t.co twitter.com|facebook fb.com|instagram|linkedin lnkd.in|reddit|hackernews news.ycombinator.com|youtube youtu.be'.split('|').map((g) => g.split(' '))

function source(host) {
  const h = host.toLowerCase().replace(/^www\./, '')
  for (const [name, ...hosts] of SOURCES) if ([name + '.com', ...hosts].some((d) => h === d || h.endsWith('.' + d))) return name
  const engine = /(^|\.)(google|yahoo)\.[a-z.]+$/.exec(h)
  return engine ? engine[2] : /^[\d.]+$|[:[]|localhost$/.test(h) ? 'other' : h.replace(/[^a-z0-9.-]/g, '').slice(0, 60) || 'other'
}

function pageview() {
  const ua = nav.userAgent || ''
  const mac = /Mac OS X/.test(ua)
  let p = location.pathname.replace(/(^|\/)(index|drop)\.html$/, '$1').replace(/\/edit$/, '/edit.html')
  if (!/\.html$|\/$/.test(p)) p += '/'
  let ref
  try { ref = new URL(document.referrer) } catch {}
  const dims = {
    p: de.dataset.pv || (/^\/(app\/(edit\.html)?|(mentions-legales|en-ligne|questions)\/)?$/.test(p) ? p : 'other'),
    e: !ref || ref.host !== location.host ? 1 : 0,
    l: /^(fr|en|es|de|it|pt|zh|ja)/.exec(de.lang)?.[1],
  }
  if (dims.e) dims.r = ref ? source(ref.hostname) : 'direct'
  const u = (new URLSearchParams(location.search).get('utm_source') || '').toLowerCase().replace(/[^a-z0-9._-]+/g, '-').replace(/^-+|-+$/g, '').slice(0, 30)
  if (u) dims.u = u
  dims.d = matchMedia('(max-width: 767px)').matches ? 'mobile' : 'desktop'
  dims.os = /CrOS/.test(ua) ? 'chromeos' : /Android/i.test(ua) ? 'android'
    : /iP(hone|ad|od)/.test(ua) || (mac && nav.maxTouchPoints > 1) ? 'ios' : mac ? 'mac'
    : /Windows/.test(ua) ? 'windows' : /Linux/.test(ua) ? 'linux' : 'other'
  track('pv', dims)
}

if (opt) try { for (const a of document.querySelectorAll('a[href^="/telecharger/"]')) a.search += '&nc=1' } catch {}
if (!off && globalThis.document) {
  // Framed: no page view
  try { if (top === self && !de.hasAttribute('data-no-pageview')) pageview() } catch {}
  const onClick = (ev) => {
    const el = ev.target instanceof Element && ev.target.closest('[data-track="gh"]')
    const src = el && el.dataset.trackSrc
    if (el && ev.button < 2) track('gh', { src: /^(header|footer|opensource|download|faq)$/.test(src) ? src : 'other' })
  }
  document.addEventListener('click', onClick, true)
  document.addEventListener('auxclick', onClick, true)
}
