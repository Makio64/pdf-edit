// The dictionaries of site/i18n (docs/WEB.md, « Languages »): the build checks them in depth
// (scripts/build-web.sh); here, what the counts rely on and what a translator most often misses.
import assert from 'node:assert/strict'
import { readdirSync, readFileSync } from 'node:fs'
import { describe, it } from 'node:test'
import { LANGS } from '../netlify/lib/validate.mts'

const DIR = new URL('../i18n/', import.meta.url)
const META = ['lang', 'og_locale', 'name', 'slugs']
const load = (code: string): Record<string, unknown> => JSON.parse(readFileSync(new URL(`${code}.json`, DIR), 'utf8'))
const codes = readdirSync(DIR)
  .filter((f) => f.endsWith('.json'))
  .map((f) => f.slice(0, -5))
const texts = (d: Record<string, unknown>) => Object.keys(d).filter((k) => !META.includes(k) && !k.startsWith('_')).sort()
const fr = load('fr')

describe('site/i18n dictionaries', () => {
  it('has French and English, the languages that are always built', () => {
    assert.ok(codes.includes('fr') && codes.includes('en'), codes.join(', '))
  })

  for (const code of codes) {
    it(`${code}.json: a language the counts accept, its meta, the keys of fr.json`, () => {
      const d = load(code)
      assert.ok((LANGS as readonly string[]).includes(code), `${code} is not in LANGS of netlify/lib/validate.mts`)
      assert.match(String(d.lang), new RegExp(`^${code}(-[A-Za-z0-9]+)*$`))
      assert.match(String(d.og_locale), /^[a-z]{2}_[A-Z]{2}$/)
      assert.ok(typeof d.name === 'string' && d.name.trim())
      const slugs = d.slugs as Record<string, string>
      assert.deepEqual(Object.keys(slugs).sort(), ['faq', 'online', 'privacy'])
      for (const slug of Object.values(slugs)) assert.match(slug, /^[a-z0-9]+(-[a-z0-9]+)*$/)
      assert.deepEqual(texts(d), texts(fr))
      for (const key of texts(d)) assert.ok(typeof d[key] === 'string' && (d[key] as string).trim(), `${code}: ${key}`)
      // A FAQ question is one flex item of <summary>: any tag (even a nowrap span) would split it.
      for (const key of texts(d).filter((k) => /^faq\.\d+\.q$/.test(k))) assert.doesNotMatch(d[key] as string, /</, `${code}: ${key}`)
    })
  }

  it('keeps the French addresses, which search engines know', () => {
    assert.deepEqual(fr.slugs, { online: 'en-ligne', faq: 'questions', privacy: 'mentions-legales' })
  })
})
