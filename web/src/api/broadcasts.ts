import { api, unwrap } from './client.ts'
import type { BroadcastInput } from './types.ts'

// The list is served with `Cache-Control: max-age=5`, so right after this tab writes,
// the browser could replay a list that still shows the old row.
let listMayBeStale = false

async function write<T>(pending: Promise<T>): Promise<T> {
  const result = await pending
  listMayBeStale = true
  return result
}

export const broadcastApi = {
  list: (before?: string) => {
    const cache = listMayBeStale ? 'no-cache' : 'default'
    listMayBeStale = false
    return unwrap(api.GET('/api/broadcasts', { params: { query: { before } }, cache }))
  },
  get: (id: string) => unwrap(api.GET('/api/broadcasts/{id}', { params: { path: { id } } })),
  create: (input: BroadcastInput) => write(unwrap(api.POST('/api/broadcasts', { body: input }))),
  update: (id: string, version: number, input: BroadcastInput) =>
    write(
      unwrap(
        api.PUT('/api/broadcasts/{id}', {
          params: { path: { id }, header: { 'If-Match': `"${version}"` } },
          body: input,
        }),
      ),
    ),
  remove: (id: string) => write(unwrap(api.DELETE('/api/broadcasts/{id}', { params: { path: { id } } }))),
  start: (id: string) => write(unwrap(api.POST('/api/broadcasts/{id}/start', { params: { path: { id } } }))),
  stop: (id: string) => write(unwrap(api.POST('/api/broadcasts/{id}/stop', { params: { path: { id } } }))),
}
