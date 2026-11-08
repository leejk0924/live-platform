import { ApiError } from '../api/client.ts'
import type { Broadcast } from '../api/types.ts'

// `refreshFailed`: the last ask failed after an earlier one succeeded, so `broadcast` may be stale.
export type BroadcastState =
  | { status: 'loading' }
  | { status: 'failed'; error: unknown }
  | { status: 'loaded'; broadcast: Broadcast; refreshFailed: boolean }

export const loaded = (broadcast: Broadcast): BroadcastState => ({ status: 'loaded', broadcast, refreshFailed: false })

// One failed refresh must not take down what is on screen: the player would stop and the owner's
// publication would close. Only a broadcast that is gone replaces it, and a broadcast can be deleted
// only while nothing is on air.
export function failed(prev: BroadcastState, id: string, error: unknown): BroadcastState {
  const gone = error instanceof ApiError && error.code === 'BROADCAST_NOT_FOUND'
  if (prev.status === 'loaded' && prev.broadcast.id === id && !gone) return { ...prev, refreshFailed: true }
  return { status: 'failed', error }
}

export const reloading = (prev: BroadcastState): BroadcastState => (prev.status === 'failed' ? { status: 'loading' } : prev)
