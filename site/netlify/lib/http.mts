import { createHash, timingSafeEqual } from 'node:crypto'

export const MAX_BODY_BYTES = 1024

const BASE_HEADERS = {
  'cache-control': 'no-store',
  'x-content-type-options': 'nosniff',
  'x-robots-tag': 'noindex',
}

export function json(status: number, body: unknown, headers: Record<string, string> = {}): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { ...BASE_HEADERS, 'content-type': 'application/json; charset=utf-8', ...headers },
  })
}

export function empty(status: number, headers: Record<string, string> = {}): Response {
  return new Response(null, { status, headers: { ...BASE_HEADERS, ...headers } })
}

/**
 * The page that sent the event must be served by this site (same host as the
 * request), or be a local development page (localhost / 127.0.0.1, any port).
 * Origin is preferred; Referer is used only when Origin is absent or "null".
 */
export function originAllowed(req: Request): boolean {
  const origin = req.headers.get('origin')
  const source = origin && origin !== 'null' ? origin : req.headers.get('referer')
  if (!source) return false
  let from: URL
  try {
    from = new URL(source)
  } catch {
    return false
  }
  if (from.protocol !== 'https:' && from.protocol !== 'http:') return false
  if (from.hostname === 'localhost' || from.hostname === '127.0.0.1') return true
  return from.host.toLowerCase() === new URL(req.url).host.toLowerCase()
}

/** Reads at most `max` bytes of the body; null when it is larger or not valid UTF-8. */
export async function readBodyCapped(req: Request, max = MAX_BODY_BYTES): Promise<string | null> {
  const declared = Number(req.headers.get('content-length'))
  if (Number.isFinite(declared) && declared > max) return null
  if (!req.body) return ''
  const reader = req.body.getReader()
  const chunks: Uint8Array[] = []
  let size = 0
  for (;;) {
    const { done, value } = await reader.read()
    if (done) break
    size += value.byteLength
    if (size > max) {
      await reader.cancel().catch(() => {})
      return null
    }
    chunks.push(value)
  }
  const bytes = new Uint8Array(size)
  let offset = 0
  for (const chunk of chunks) {
    bytes.set(chunk, offset)
    offset += chunk.byteLength
  }
  try {
    return new TextDecoder('utf-8', { fatal: true }).decode(bytes)
  } catch {
    return null
  }
}

export function bearerToken(req: Request): string | null {
  const match = /^Bearer[ \t]+(\S+)[ \t]*$/i.exec(req.headers.get('authorization') ?? '')
  return match ? match[1] : null
}

/** Compares two secrets in constant time (hashing first so lengths never leak). */
export function constantTimeEqual(a: string, b: string): boolean {
  const ha = createHash('sha256').update(a, 'utf8').digest()
  const hb = createHash('sha256').update(b, 'utf8').digest()
  return timingSafeEqual(ha, hb)
}

/** The browser asks not to be tracked: Global Privacy Control (`Sec-GPC: 1`) or Do Not Track (`DNT: 1`). */
export function asksNotToBeTracked(req: Request): boolean {
  return req.headers.get('sec-gpc')?.trim() === '1' || req.headers.get('dnt')?.trim() === '1'
}

export function mediaType(req: Request): string {
  return (req.headers.get('content-type') ?? '').split(';')[0].trim().toLowerCase()
}
