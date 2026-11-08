import { useState } from 'react'
import { Link, useNavigate, useParams } from 'react-router'
import { broadcastApi } from '../api/broadcasts.ts'
import { errorMessage } from '../api/client.ts'
import type { Broadcast } from '../api/types.ts'
import { useAuth } from '../auth/context.ts'
import LoadFailed from '../broadcasts/LoadFailed.tsx'
import { statusLabel } from '../broadcasts/status.ts'
import { useBroadcast } from '../broadcasts/useBroadcast.ts'

export default function BroadcastDetail() {
  const { id = '' } = useParams()
  const { state } = useBroadcast(id)

  if (state.status === 'loading') return <p>불러오는 중…</p>
  if (state.status === 'failed') return <LoadFailed error={state.error} />
  return <BroadcastView broadcast={state.broadcast} />
}

function BroadcastView({ broadcast }: { broadcast: Broadcast }) {
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
