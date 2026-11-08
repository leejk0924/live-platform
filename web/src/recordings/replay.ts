// Where to pick up after a new URL loads: the position and whether it was playing.
export type Resume = { time: number; playing: boolean }

// `retried`: the URL in hand replaced one that failed, and nothing has loaded since. Another media
// error then gives up instead of asking again, so a recording that cannot play does not loop.
// `error` is the API error when asking for a URL failed, `null` when the media itself failed.
export type ReplayState =
  | { kind: 'loading'; resume: Resume | null; retried: boolean }
  | { kind: 'ready'; url: string; resume: Resume | null; retried: boolean }
  | { kind: 'failed'; resume: Resume | null; error: unknown }

export type ReplayEvent =
  | { type: 'urlLoaded'; url: string }
  | { type: 'urlFailed'; error: unknown }
  | { type: 'mediaLoaded' }
  | { type: 'mediaError'; at: Resume }
  | { type: 'retry' }

export const initialReplay: ReplayState = { kind: 'loading', resume: null, retried: false }

export function replayReducer(state: ReplayState, event: ReplayEvent): ReplayState {
  switch (event.type) {
    case 'urlLoaded':
      return state.kind === 'loading' ? { ...state, kind: 'ready', url: event.url } : state
    case 'urlFailed':
      return state.kind === 'loading' ? { kind: 'failed', resume: state.resume, error: event.error } : state
    case 'mediaLoaded':
      return state.kind === 'ready' ? { ...state, resume: null, retried: false } : state
    case 'mediaError': {
      if (state.kind !== 'ready') return state
      // A URL that never loaded leaves the earlier position as the one to come back to.
      const resume = state.resume ?? event.at
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
