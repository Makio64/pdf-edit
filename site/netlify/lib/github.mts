// Download counts of the .dmg assets published on GitHub Releases (public API, no token).
// Cached for an hour in the stats store under `c/github`; any failure gives null.

import { GITHUB_CACHE_KEY } from './keys.mts'
import type { StatsStore } from './store.mts'

export interface GithubStats {
  total: number
  assets: { name: string; downloads: number }[]
  fetchedAt: string
}

interface CacheEntry {
  repo: string
  fetchedAt: string
  /** null when the last attempt failed (retried after FAILURE_TTL_MS). */
  data: GithubStats | null
}

export const CACHE_TTL_MS = 60 * 60_000
/** GitHub's unauthenticated quota is shared by every function on the same IP: back off after a failure. */
export const FAILURE_TTL_MS = 10 * 60_000
const REPO_RE = /^[A-Za-z0-9_.-]{1,100}\/[A-Za-z0-9_.-]{1,100}$/

type FetchLike = (url: string, init?: RequestInit) => Promise<Response>

export function summariseReleases(releases: unknown, nowMs: number): GithubStats | null {
  if (!Array.isArray(releases)) return null
  const assets: GithubStats['assets'] = []
  for (const release of releases) {
    const list = release && typeof release === 'object' ? (release as { assets?: unknown }).assets : null
    if (!Array.isArray(list)) continue
    for (const asset of list) {
      if (!asset || typeof asset !== 'object') continue
      const { name, download_count } = asset as { name?: unknown; download_count?: unknown }
      if (typeof name !== 'string' || !name.toLowerCase().endsWith('.dmg')) continue
      const downloads = typeof download_count === 'number' && download_count > 0 ? Math.floor(download_count) : 0
      assets.push({ name, downloads })
    }
  }
  assets.sort((a, b) => b.downloads - a.downloads || (a.name < b.name ? -1 : 1))
  return { total: assets.reduce((sum, a) => sum + a.downloads, 0), assets, fetchedAt: new Date(nowMs).toISOString() }
}

export async function githubDownloads(
  store: StatsStore,
  repo: string | null | undefined,
  { fetch: fetchImpl = fetch as FetchLike, nowMs = Date.now() }: { fetch?: FetchLike; nowMs?: number } = {},
): Promise<GithubStats | null> {
  if (!repo || !REPO_RE.test(repo)) return null

  try {
    const cached = (await store.get(GITHUB_CACHE_KEY, { type: 'json' })) as CacheEntry | null
    if (cached && cached.repo === repo) {
      const age = nowMs - Date.parse(cached.fetchedAt)
      if (age >= 0 && age < (cached.data ? CACHE_TTL_MS : FAILURE_TTL_MS)) return cached.data
    }
  } catch {
    // Unreadable cache: fetch again.
  }

  let data: GithubStats | null = null
  try {
    const res = await fetchImpl(`https://api.github.com/repos/${repo}/releases?per_page=100`, {
      headers: {
        accept: 'application/vnd.github+json',
        'user-agent': 'pdf-edit-stats',
        'x-github-api-version': '2022-11-28',
      },
      signal: AbortSignal.timeout(5000),
    })
    if (res.ok) data = summariseReleases(await res.json(), nowMs)
    else await res.body?.cancel()
  } catch {
    data = null
  }

  const entry: CacheEntry = { repo, fetchedAt: new Date(nowMs).toISOString(), data }
  try {
    await store.set(GITHUB_CACHE_KEY, JSON.stringify(entry))
  } catch {
    // The cache is an optimisation only.
  }
  return data
}
