import { api, unwrap } from './client.ts'
import type { BroadcastInput } from './types.ts'

// The list is served with `Cache-Control: max-age=5`, so for that long after this tab writes,
// the browser could replay a list that still shows the old row. Every list request in that window
// revalidates, since Home asks for two at once.
const LIST_MAX_AGE_MS = 5000
let listStaleUntil = 0

async function write<T>(pending: Promise<T>): Promise<T> {
  const result = await pending
  listStaleUntil = Date.now() + LIST_MAX_AGE_MS
  return result
}

export const broadcastApi = {
  list: (query: { live?: boolean; before?: string }) => {
    const cache = Date.now() < listStaleUntil ? 'no-cache' : 'default'
    return unwrap(api.GET('/api/broadcasts', { params: { query }, cache }))
  },
  get: (id: string) => unwrap(api.GET('/api/broadcasts/{id}', { params: { path: { id } } })),
  playback: (id: string) => unwrap(api.GET('/api/broadcasts/{id}/playback', { params: { path: { id } } })),
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
