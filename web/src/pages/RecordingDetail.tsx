import { useCallback, useEffect, useState } from 'react'
import { Link, useParams } from 'react-router'
import { recordingApi } from '../api/recordings.ts'
import type { Recording } from '../api/types.ts'
import LoadFailed from '../broadcasts/LoadFailed.tsx'
import { RecordingFacts } from '../recordings/RecordingItem.tsx'
import ReplayPlayer from '../recordings/ReplayPlayer.tsx'

type Load = { status: 'loading' } | { status: 'failed'; error: unknown } | { status: 'loaded'; recording: Recording }

// Keyed by id so moving to another recording starts from loading instead of showing the last one.
export default function RecordingDetail() {
  const { id = '' } = useParams()
  return <RecordingPage key={id} id={id} />
}

function RecordingPage({ id }: { id: string }) {
  const [load, setLoad] = useState<Load>({ status: 'loading' })
  const [attempt, setAttempt] = useState(0)

  useEffect(() => {
    let active = true
    recordingApi.get(id).then(
      (recording) => active && setLoad({ status: 'loaded', recording }),
      (error: unknown) => active && setLoad({ status: 'failed', error }),
    )
    return () => {
      active = false
    }
  }, [id, attempt])

  const reload = useCallback(() => {
    setLoad({ status: 'loading' })
    setAttempt((n) => n + 1)
  }, [])

  if (load.status === 'loading') return <p>불러오는 중…</p>
  if (load.status === 'failed') return <LoadFailed error={load.error} onRetry={reload} subject="recording" />
  const { recording } = load
  return (
    <article className="stack">
      <h1 className="broadcast-heading">{recording.broadcastTitle}</h1>
      <p className="broadcast-meta">
        {recording.ownerName} · <RecordingFacts recording={recording} />
      </p>
      <Screen recording={recording} onRefresh={reload} />
      <p>
        <Link to={`/broadcasts/${recording.broadcastId}`}>방송 페이지로</Link>
        {' · '}
        <Link to="/recordings">다시보기 목록</Link>
      </p>
    </article>
  )
}

function Screen({ recording, onRefresh }: { recording: Recording; onRefresh: () => void }) {
  switch (recording.status) {
    case 'READY':
      return <ReplayPlayer key={recording.id} recordingId={recording.id} />
    case 'PENDING':
      return (
        <div className="stack">
          <p className="notice">녹화를 저장하는 중입니다. 잠시 후 새로고침하세요.</p>
          <div className="actions">
            <button type="button" onClick={onRefresh}>
              새로고침
            </button>
          </div>
        </div>
      )
    case 'FAILED':
      return <p className="notice">녹화를 저장하지 못해 다시 볼 수 없습니다.</p>
  }
}
