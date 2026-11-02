// POST /api/e: cookieless analytics ingestion. Always 204, never reveals whether an event was kept.
import { getStore } from '@netlify/blobs'
import type { Config, Context } from '@netlify/functions'
import { handleEvent } from '../lib/handlers.mts'
import { STATS_REGION, storeName } from '../lib/store.mts'

export default async (req: Request, context: Context): Promise<Response> =>
  handleEvent(req, {
    store: () => getStore({ name: storeName(context.deploy?.context), consistency: 'strong', region: STATS_REGION }),
    waitUntil: (promise) => context.waitUntil(promise),
  })

export const config: Config = {
  path: '/api/e',
  region: 'fra',
  method: ['POST'],
  rateLimit: { windowLimit: 60, windowSize: 60, aggregateBy: ['ip', 'domain'] },
}
