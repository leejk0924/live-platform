import type { BroadcastStatus } from '../api/types.ts'
import { statusLabel } from './status.ts'

export default function StatusBadge({ status }: { status: BroadcastStatus }) {
  return <span className={`badge badge-${status}`}>{statusLabel[status]}</span>
}
