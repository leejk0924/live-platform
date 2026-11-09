import type { BroadcastStatus } from '../../api/types.ts'
import type { SendStats } from './whip.ts'

export type Phase =
  | { kind: 'idle' }
  | { kind: 'capturing' }
  | { kind: 'connecting' }
  | { kind: 'publishing'; stats: SendStats | null }
  | { kind: 'paused' }
  | { kind: 'stopping' }

export type BackgroundAction = 'pause' | 'resume' | 'idle' | null

// Mobile browsers stop the camera in the background. Leaving lets the media server mark the
// broadcast INTERRUPTED, and coming back within the 30 s grace publishes again with a fresh token.
export function backgroundAction({
  phase,
  hidden,
  pauses,
  status,
}: {
  phase: Phase['kind']
  hidden: boolean
  pauses: boolean
  status: BroadcastStatus
}): BackgroundAction {
  if (!pauses) return null
  if (hidden && phase === 'publishing') return 'pause'
  // Past the grace the broadcast is over, and starting would open the camera only to be refused.
  if (!hidden && phase === 'paused') return status === 'ENDED' ? 'idle' : 'resume'
  return null
}
