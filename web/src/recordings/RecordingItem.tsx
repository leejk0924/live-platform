import { Link } from 'react-router'
import type { Recording, RecordingStatus } from '../api/types.ts'
import { formatDuration, formatSize } from './replay.ts'

// A ready recording needs no badge: it is the usual case.
const recordingStatusLabel: Record<RecordingStatus, string | null> = {
  PENDING: '처리 중',
  READY: null,
  FAILED: '실패',
}

export function RecordingFacts({ recording }: { recording: Recording }) {
  const label = recordingStatusLabel[recording.status]
  return (
    <>
      <time dateTime={recording.recordedAt}>{new Date(recording.recordedAt).toLocaleString('ko-KR')}</time>
      {' · '}
      {formatDuration(recording.durationMs)}
      {recording.sizeBytes !== null && ` · ${formatSize(recording.sizeBytes)}`}
      {label && (
        <>
          {' · '}
          <span className={`badge badge-${recording.status}`}>{label}</span>
        </>
      )}
    </>
  )
}

// `withTitle` is off where the broadcast is already on screen.
export default function RecordingItem({ recording, withTitle }: { recording: Recording; withTitle: boolean }) {
  return (
    <li className={`status-${recording.status}`}>
      <Link to={`/recordings/${recording.id}`}>
        {withTitle && <span className="broadcast-title">{recording.broadcastTitle}</span>}
        <span className="broadcast-meta">
          {withTitle && `${recording.ownerName} · `}
          <RecordingFacts recording={recording} />
        </span>
      </Link>
    </li>
  )
}
