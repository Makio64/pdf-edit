// In-memory StatsStore with the same list/set semantics as Netlify Blobs
// (prefix filter, `directories: true` grouping, `onlyIfNew`). Used by the tests.

import type { ListResult, StatsStore } from './store.mts'

export interface MemoryStore extends StatsStore {
  data: Map<string, string>
  calls: { set: string[]; get: string[]; list: string[]; pages: number }
}

/** Keys per page when listing with `paginate: true` (Netlify Blobs sends up to 1000). */
export const PAGE_SIZE = 1000

export function memoryStore(initial: Record<string, string> = {}): MemoryStore {
  const data = new Map<string, string>(Object.entries(initial))
  const calls = { set: [] as string[], get: [] as string[], list: [] as string[], pages: 0 }
  return {
    data,
    calls,
    async set(key, value, options = {}) {
      calls.set.push(key)
      if (options.onlyIfNew && data.has(key)) return { modified: false }
      data.set(key, String(value))
      return { modified: true, etag: String(data.size) }
    },
    async get(key, options) {
      calls.get.push(key)
      const value = data.get(key)
      if (value === undefined) return null
      return options?.type === 'json' ? JSON.parse(value) : value
    },
    list({ prefix = '', directories = false, paginate = false }: { prefix?: string; directories?: boolean; paginate?: boolean } = {}): any {
      calls.list.push(prefix)
      const blobs: { key: string }[] = []
      const dirs = new Set<string>()
      for (const key of [...data.keys()].sort()) {
        if (!key.startsWith(prefix)) continue
        const rest = key.slice(prefix.length)
        const slash = rest.indexOf('/')
        if (directories && slash !== -1) dirs.add(prefix + rest.slice(0, slash))
        else blobs.push({ key })
      }
      if (!paginate) return Promise.resolve({ blobs, directories: [...dirs] } satisfies ListResult)
      // Like Netlify Blobs: pages of PAGE_SIZE keys, fetched one after the other as the loop asks.
      return (async function* pages(): AsyncGenerator<ListResult> {
        for (let i = 0; i === 0 || i < blobs.length; i += PAGE_SIZE) {
          calls.pages++
          yield { blobs: blobs.slice(i, i + PAGE_SIZE), directories: i === 0 ? [...dirs] : [] }
        }
      })()
    },
  }
}
