import { useEffect, useState } from 'react'
import { Link, useNavigate, useParams } from 'react-router'
import { broadcastApi } from '../api/broadcasts.ts'
import { errorMessage } from '../api/client.ts'
import type { Broadcast } from '../api/types.ts'
import { useAuth } from '../auth/context.ts'
import LoadFailed from '../broadcasts/LoadFailed.tsx'
import CommentPanel from '../comments/CommentPanel.tsx'
import DonationPanel from '../donations/DonationPanel.tsx'
import { useDonations } from '../donations/useDonations.ts'
import BroadcastRecordings from '../recordings/BroadcastRecordings.tsx'
import Publisher from '../broadcasts/publish/Publisher.tsx'
import { statusLabel } from '../broadcasts/status.ts'
import { useBroadcast } from '../broadcasts/useBroadcast.ts'
import Player from '../broadcasts/watch/Player.tsx'

const REFRESH_MS = 5000

export default function BroadcastDetail() {
  const { id = '' } = useParams()
  const { state, reload } = useBroadcast(id)

  // Viewers learn about start, interruption and end only by asking again. The next ask waits for the
  // previous answer, since a new ask discards the one in flight: on a fixed interval, an answer slower
  // than the interval would never land. A failed first load stops asking and offers a retry instead;
  // a failed refresh keeps the page and asks again.
  useEffect(() => {
    if (state.status !== 'loaded' || state.broadcast.status === 'ENDED') return
    const timer = window.setTimeout(reload, REFRESH_MS)
    return () => window.clearTimeout(timer)
  }, [state, reload])

  if (state.status === 'loading') return <p>불러오는 중…</p>
  if (state.status === 'failed') return <LoadFailed error={state.error} onRetry={reload} />
  return (
    <>
      {state.refreshFailed && (
        <p className="notice" role="status">
          상태를 새로 고치지 못했습니다. 잠시 후 다시 시도합니다.
        </p>
      )}
      <BroadcastView key={state.broadcast.id} broadcast={state.broadcast} onChanged={reload} />
    </>
  )
}

function BroadcastView({ broadcast, onChanged }: { broadcast: Broadcast; onChanged: () => void }) {
  const { auth } = useAuth()
  const navigate = useNavigate()
  const [pending, setPending] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const isOwner = auth.status === 'signedIn' && auth.user.id === broadcast.ownerId
  const donations = useDonations(broadcast.id)

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
      <DonationPanel
        broadcast={broadcast}
        state={donations.state}
        loadError={donations.loadError}
        onRetry={donations.retry}
      />
      {broadcast.status === 'ENDED' && <BroadcastRecordings key={broadcast.id} broadcastId={broadcast.id} />}
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
      <CommentPanel key={broadcast.id} broadcast={broadcast} onDonations={donations.dispatch} />
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
