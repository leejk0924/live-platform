import { api, unwrap } from './client.ts'

export const recordingApi = {
  list: (query: { broadcastId?: string; before?: string; limit?: number }) =>
    unwrap(api.GET('/api/recordings', { params: { query } })),
  get: (id: string) => unwrap(api.GET('/api/recordings/{id}', { params: { path: { id } } })),
  replay: (id: string) => unwrap(api.GET('/api/recordings/{id}/replay', { params: { path: { id } } })),
}
