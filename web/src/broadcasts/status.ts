import type { BroadcastStatus } from '../api/types.ts'

export const statusLabel: Record<BroadcastStatus, string> = {
  READY: '준비',
  LIVE: '방송 중',
  INTERRUPTED: '연결 끊김',
  ENDED: '종료',
}
