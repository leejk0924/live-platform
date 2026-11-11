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
    <li className="card">
      <Link to={`/recordings/${recording.id}`} className="card-link">
        <span className={`thumb thumb-${recording.status}`} aria-hidden="true">
          <svg className="thumb-icon" viewBox="0 0 24 24">
            <circle cx="12" cy="12" r="9" />
            <path d="m10 8.5 5 3.5-5 3.5z" />
          </svg>
          <span className="thumb-duration">{formatDuration(recording.durationMs)}</span>
        </span>
        <span className="card-body">
          {withTitle && <span className="card-title">{recording.broadcastTitle}</span>}
          <span className="card-meta">
            {withTitle && `${recording.ownerName} · `}
            <RecordingFacts recording={recording} />
          </span>
        </span>
      </Link>
    </li>
  )
}
