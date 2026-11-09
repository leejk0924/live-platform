import type { BroadcastStatus } from '../../api/types.ts'
import type { SendStats } from './whip.ts'

export type Phase =
  | { kind: 'idle' }
  | { kind: 'capturing' }
  | { kind: 'connecting' }
  | { kind: 'publishing'; stats: SendStats | null }
  | { kind: 'paused'; since: number }
  | { kind: 'stopping' }

// The media server's grace (ADR media/10): an INTERRUPTED broadcast ends after this long.
export const GRACE_MS = 30_000

export type BackgroundAction = 'pause' | 'resume' | 'idle' | null

// Mobile browsers stop the camera in the background. Leaving lets the media server mark the
// broadcast INTERRUPTED, and coming back within the 30 s grace publishes again with a fresh token.
export function backgroundAction({
  phase,
  hidden,
  pauses,
  status,
  pausedMs,
}: {
  phase: Phase['kind']
  hidden: boolean
  pauses: boolean
  status: BroadcastStatus
  pausedMs: number
}): BackgroundAction {
  if (!pauses) return null
  if (hidden && phase === 'publishing') return 'pause'
  // Past the grace the broadcast is over, and starting would open the camera only to be refused.
  // A page suspended in the background never saw the status change, so the time away decides too.
  if (!hidden && phase === 'paused') return status === 'ENDED' || pausedMs >= GRACE_MS ? 'idle' : 'resume'
  return null
}
