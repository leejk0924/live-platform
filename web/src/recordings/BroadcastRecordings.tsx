import { useEffect, useState } from 'react'
import { errorMessage } from '../api/client.ts'
import { recordingApi } from '../api/recordings.ts'
import type { Recording } from '../api/types.ts'
import RecordingItem from './RecordingItem.tsx'

// A broadcast has one recording per publisher connection and per 30 minutes, so one page of the
// largest size covers all but a very long broadcast.
const LIMIT = 50

export default function BroadcastRecordings({ broadcastId }: { broadcastId: string }) {
  const [recordings, setRecordings] = useState<Recording[]>([])
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    let active = true
    recordingApi.list({ broadcastId, limit: LIMIT }).then(
      (page) => active && setRecordings(page.items),
      (e: unknown) => active && setError(errorMessage(e)),
    )
    return () => {
      active = false
    }
  }, [broadcastId])

  if (error) {
    return (
      <p className="error" role="alert">
        다시보기를 불러오지 못했습니다. {error}
      </p>
    )
  }
  if (recordings.length === 0) return null
  return (
    <section className="feed" aria-label="다시보기">
      <h2>다시보기</h2>
      <ul className="card-grid">
        {recordings.map((r) => (
          <RecordingItem key={r.id} recording={r} withTitle={false} />
        ))}
      </ul>
    </section>
  )
}
