import { useEffect, useState } from 'react'
import { Link, useNavigate, useParams } from 'react-router'
import { broadcastApi } from '../api/broadcasts.ts'
import { errorMessage } from '../api/client.ts'
import type { Broadcast } from '../api/types.ts'
import { useAuth } from '../auth/context.ts'
import LoadFailed from '../broadcasts/LoadFailed.tsx'
import Publisher from '../broadcasts/publish/Publisher.tsx'
import { statusLabel } from '../broadcasts/status.ts'
import { useBroadcast } from '../broadcasts/useBroadcast.ts'
import Player from '../broadcasts/watch/Player.tsx'

const REFRESH_MS = 5000

export default function BroadcastDetail() {
  const { id = '' } = useParams()
  const { state, reload } = useBroadcast(id)
  const ended = state.status === 'loaded' && state.broadcast.status === 'ENDED'

  // Viewers learn about start, interruption and end only by asking again.
  useEffect(() => {
    if (ended) return
    const timer = window.setInterval(reload, REFRESH_MS)
    return () => window.clearInterval(timer)
  }, [ended, reload])

  if (state.status === 'loading') return <p>불러오는 중…</p>
  if (state.status === 'failed') return <LoadFailed error={state.error} />
  return <BroadcastView broadcast={state.broadcast} onChanged={reload} />
}

function BroadcastView({ broadcast, onChanged }: { broadcast: Broadcast; onChanged: () => void }) {
  const { auth } = useAuth()
  const navigate = useNavigate()
  const [pending, setPending] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const isOwner = auth.status === 'signedIn' && auth.user.id === broadcast.ownerId

  async function onDelete() {
    if (!window.confirm('이 방송을 삭제할까요?')) return
    setPending(true)
    setError(null)
    try {
      await broadcastApi.remove(broadcast.id)
      navigate('/', { replace: true })
    } catch (e) {
      setError(errorMessage(e))
      setPending(false)
    }
  }

  return (
    <article className="stack">
      <h1 className="broadcast-heading">{broadcast.title}</h1>
      <p className="broadcast-meta">
        {broadcast.ownerName} · <span className={`badge badge-${broadcast.status}`}>{statusLabel[broadcast.status]}</span>
        {' · '}
        <time dateTime={broadcast.createdAt}>{new Date(broadcast.createdAt).toLocaleString('ko-KR')}</time>
      </p>
      {broadcast.description && <p className="description">{broadcast.description}</p>}
      <Screen broadcast={broadcast} onLost={onChanged} />
      {isOwner && <Publisher broadcast={broadcast} onChanged={onChanged} />}
      {isOwner && (
        <div className="actions">
          <Link to={`/broadcasts/${broadcast.id}/edit`} className="button">
            수정
          </Link>
          <button type="button" onClick={onDelete} disabled={pending}>
            삭제
          </button>
        </div>
      )}
      {error && (
        <p className="error" role="alert">
          {error}
        </p>
      )}
    </article>
  )
}

function Screen({ broadcast, onLost }: { broadcast: Broadcast; onLost: () => void }) {
  switch (broadcast.status) {
    case 'READY':
      return <p className="notice">아직 방송 전입니다.</p>
    case 'LIVE':
      return <Player key={broadcast.id} broadcastId={broadcast.id} onLost={onLost} />
    case 'INTERRUPTED':
      return <p className="notice">방송 연결이 끊겼습니다. 다시 연결되면 이어서 볼 수 있습니다.</p>
    case 'ENDED':
      return <p className="notice">종료된 방송입니다.</p>
  }
}
