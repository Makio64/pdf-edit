// Event allowlist for the cookieless analytics (see docs/WEB.md).
// Anything that is not listed here is dropped silently by /api/e.

export type Dims = Record<string, string>
export interface ValidEvent {
  n: string
  dims: Dims
}

interface Rule {
  values?: readonly string[]
  re?: RegExp
  required?: boolean
}
interface EventRule {
  dims: Record<string, Rule>
  /** Only the server may record it (never accepted from the browser). */
  serverOnly?: boolean
}

// The pages the beacon names: the French address of the page whatever its language (each public page
// declares it in <html data-pv>; otherwise src/assets/track.js normalises the path to one of them). /app/
// and /app/edit.html are only counted when opened on their own: framed in /en-ligne/ they send no page view.
export const PAGES = ['/', '/en-ligne/', '/questions/', '/mentions-legales/', '/app/', '/app/edit.html', '/404', 'other'] as const
// The languages of the site (the dictionaries of site/i18n; scripts/build-web.sh refuses one that is not
// listed here): `l` of a page view is the page's language, from <html lang>.
export const LANGS = ['fr', 'en', 'es', 'de', 'it', 'pt', 'zh', 'ja'] as const
export const OS_LIST = ['mac', 'ios', 'windows', 'android', 'linux', 'chromeos', 'other'] as const
export const DL_SOURCES = ['hero', 'header', 'download', 'footer', 'faq', 'app', 'other'] as const
export const GH_SOURCES = ['header', 'footer', 'opensource', 'download', 'faq', 'other'] as const
const BIT = ['0', '1'] as const

export const EVENTS: Record<string, EventRule> = {
  pv: {
    dims: {
      p: { values: PAGES, required: true },
      e: { values: BIT },
      // A host name or a group, never an IP address (which could be the visitor's own): at least one letter.
      r: { re: /^(?=[0-9.-]*[a-z])[a-z0-9.-]{1,60}$/ },
      u: { re: /^[a-z0-9._-]{1,30}$/ },
      d: { values: ['mobile', 'desktop'] },
      os: { values: OS_LIST },
      l: { values: LANGS },
    },
  },
  conv: {
    dims: {
      f: { values: ['pages', 'docx'], required: true },
      ok: { values: BIT, required: true },
      t: { values: ['lt5', '5-15', '15-60', 'gt60'] },
      err: { values: ['other'] },
    },
  },
  pw: { dims: { w: { values: ['conv', 'edit'] } } },
  eopen: { dims: { ok: { values: BIT } } },
  esave: { dims: { ok: { values: BIT } } },
  gh: { dims: { src: { values: GH_SOURCES } } },
  dl: {
    serverOnly: true,
    dims: {
      v: { re: /^[0-9][0-9a-z.-]{0,15}$/ },
      src: { values: DL_SOURCES },
      os: { values: OS_LIST },
      // The language of the page the link was on (`&l=` of the translated pages' links); none: French.
      l: { values: LANGS },
    },
  },
}

export const EVENT_NAMES = Object.keys(EVENTS)

/** Turns a raw JSON value into the string stored in the key, or null when unusable. */
function normalise(value: unknown): string | null {
  if (typeof value === 'string') return value
  if (typeof value === 'number' && Number.isInteger(value)) return String(value)
  if (typeof value === 'boolean') return value ? '1' : '0'
  return null
}

function ruleAccepts(rule: Rule, value: string): boolean {
  if (rule.values) return rule.values.includes(value)
  if (rule.re) return rule.re.test(value)
  return false
}

/**
 * Validates `{ n, ...dims }` against the allowlist. Returns null for anything
 * unknown or malformed: unknown event, unknown dimension, bad value, missing
 * required dimension, or a server-only event when `serverSide` is false.
 * `null` / `undefined` dimension values count as absent.
 */
export function validateEvent(input: unknown, { serverSide = false } = {}): ValidEvent | null {
  if (!input || typeof input !== 'object' || Array.isArray(input)) return null
  const raw = input as Record<string, unknown>
  const n = raw.n
  if (typeof n !== 'string' || !Object.hasOwn(EVENTS, n)) return null
  const rule = EVENTS[n]
  if (rule.serverOnly && !serverSide) return null

  const dims: Dims = {}
  for (const key of Object.keys(raw)) {
    if (key === 'n') continue
    const value = raw[key]
    if (value === null || value === undefined) continue
    if (!Object.hasOwn(rule.dims, key)) return null
    const str = normalise(value)
    if (str === null || !ruleAccepts(rule.dims[key], str)) return null
    dims[key] = str
  }
  for (const [key, dimRule] of Object.entries(rule.dims)) {
    if (dimRule.required && !(key in dims)) return null
  }
  return { n, dims }
}

export const BOT_RE =
  /bot|crawl|spider|slurp|headless|lighthouse|preview|curl|wget|python|httpclient|axios|go-http|java\//i

/** Bots, scripts and requests without any user agent are never counted. */
export function isBot(ua: string | null | undefined): boolean {
  if (!ua || !ua.trim()) return true
  return BOT_RE.test(ua)
}

/** Coarse operating system from a user agent (server side: an iPad asking for a desktop site reads as mac). */
export function osFromUA(ua: string | null | undefined): (typeof OS_LIST)[number] {
  const s = ua ?? ''
  if (/CrOS/.test(s)) return 'chromeos'
  if (/Android/i.test(s)) return 'android'
  if (/iPhone|iPad|iPod/.test(s)) return 'ios'
  if (/Macintosh|Mac OS X/.test(s)) return 'mac'
  if (/Windows/.test(s)) return 'windows'
  if (/Linux/.test(s)) return 'linux'
  return 'other'
}
