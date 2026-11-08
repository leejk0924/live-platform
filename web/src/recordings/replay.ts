// Where to pick up after a new URL loads: the position and whether it was playing.
export type Resume = { time: number; playing: boolean }

// `retried`: the URL in hand replaced one that failed less than `EXPIRY_SUSPECT_MS` after it was
// loaded. Another quick media error then gives up instead of asking again, so a recording that fails
// at the same spot does not loop. An error on a URL that has been in use longer may be its expiry
// (the server signs for five minutes), so it always asks again from where playback stopped.
// `error` is the API error when asking for a URL failed, `null` when the media itself failed.
export type ReplayState =
  | { kind: 'loading'; resume: Resume | null; retried: boolean }
  | { kind: 'ready'; url: string; loadedAt: number; resume: Resume | null; retried: boolean }
  | { kind: 'failed'; resume: Resume | null; error: unknown }

export type ReplayEvent =
  | { type: 'urlLoaded'; url: string; now: number }
  | { type: 'urlFailed'; error: unknown }
  | { type: 'mediaError'; at: Resume; now: number }
  | { type: 'retry' }

const EXPIRY_SUSPECT_MS = 60_000

export const initialReplay: ReplayState = { kind: 'loading', resume: null, retried: false }

export function replayReducer(state: ReplayState, event: ReplayEvent): ReplayState {
  switch (event.type) {
    case 'urlLoaded':
      return state.kind === 'loading' ? { ...state, kind: 'ready', url: event.url, loadedAt: event.now } : state
    case 'urlFailed':
      return state.kind === 'loading' ? { kind: 'failed', resume: state.resume, error: event.error } : state
    case 'mediaError': {
      if (state.kind !== 'ready') return state
      if (event.now - state.loadedAt >= EXPIRY_SUSPECT_MS) {
        return { kind: 'loading', resume: event.at, retried: true }
      }
      // A replacement URL that fails at once leaves the position it was meant to restore.
      const resume = state.retried ? (state.resume ?? event.at) : event.at
      return state.retried ? { kind: 'failed', resume, error: null } : { kind: 'loading', resume, retried: true }
    }
    case 'retry':
      return state.kind === 'failed' ? { kind: 'loading', resume: state.resume, retried: false } : state
  }
}

const pad = (n: number) => String(n).padStart(2, '0')

export function formatDuration(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000))
  const hours = Math.floor(total / 3600)
  const minutes = Math.floor((total % 3600) / 60)
  const seconds = total % 60
  return hours > 0 ? `${hours}:${pad(minutes)}:${pad(seconds)}` : `${pad(minutes)}:${pad(seconds)}`
}

export function formatSize(bytes: number): string {
  return `${(bytes / 1_000_000).toFixed(1)} MB`
}
