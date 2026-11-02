// GET /api/stats: private aggregated counts for the /stats/ dashboard (Bearer STATS_KEY).
import { getStore } from '@netlify/blobs'
import type { Config, Context } from '@netlify/functions'
import { handleStats } from '../lib/handlers.mts'
import { STATS_REGION, storeName } from '../lib/store.mts'

export default async (req: Request, context: Context): Promise<Response> =>
  handleStats(req, {
    store: () => getStore({ name: storeName(context.deploy?.context), consistency: 'strong', region: STATS_REGION }),
    env: (name) => Netlify.env.get(name),
  })

export const config: Config = {
  path: '/api/stats',
  region: 'fra',
  method: ['GET'],
  // The dashboard asks once per range change: this only slows down someone guessing STATS_KEY.
  rateLimit: { windowLimit: 30, windowSize: 60, aggregateBy: ['ip', 'domain'] },
}
