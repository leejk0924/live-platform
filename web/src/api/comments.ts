import { api, unwrap } from './client.ts'

export const commentApi = {
  list: (id: string) => unwrap(api.GET('/api/broadcasts/{id}/comments', { params: { path: { id } } })),
  post: (id: string, body: string) =>
    unwrap(api.POST('/api/broadcasts/{id}/comments', { params: { path: { id } }, body: { body } })),
  edit: (id: string, commentId: string, version: number, body: string) =>
    unwrap(
      api.PUT('/api/broadcasts/{id}/comments/{commentId}', {
        params: { path: { id, commentId }, header: { 'If-Match': `"${version}"` } },
        body: { body },
      }),
    ),
  remove: (id: string, commentId: string) =>
    unwrap(api.DELETE('/api/broadcasts/{id}/comments/{commentId}', { params: { path: { id, commentId } } })),
  getPolicy: (id: string) => unwrap(api.GET('/api/broadcasts/{id}/chat-policy', { params: { path: { id } } })),
  setPolicy: (id: string, slowModeSeconds: number) =>
    unwrap(api.PUT('/api/broadcasts/{id}/chat-policy', { params: { path: { id } }, body: { slowModeSeconds } })),
}

// Same origin, so the session cookie goes along and CSP `connect-src 'self'` allows it.
// `after` is the list's `revision`; "0" before the first comment replays from the start.
export function eventsUrl(id: string, after: string): string {
  return `/api/broadcasts/${encodeURIComponent(id)}/events?after=${encodeURIComponent(after)}`
}
