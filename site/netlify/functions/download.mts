// GET /telecharger/mac?from=<src>: counts one Mac download (not HEAD, not bots), then 302 to the DMG.
import { getStore } from '@netlify/blobs'
import type { Config, Context } from '@netlify/functions'
import { handleDownload } from '../lib/handlers.mts'
import { release } from '../lib/release.generated.mts'
import { STATS_REGION, storeName } from '../lib/store.mts'

export default async (req: Request, context: Context): Promise<Response> =>
  handleDownload(req, {
    store: () => getStore({ name: storeName(context.deploy?.context), consistency: 'strong', region: STATS_REGION }),
    waitUntil: (promise) => context.waitUntil(promise),
    release,
  })

export const config: Config = {
  path: '/telecharger/mac',
  region: 'fra',
}
