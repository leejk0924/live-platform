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
import StatusBadge from '../broadcasts/StatusBadge.tsx'
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

  if (state.status === 'loading') {
    return (
      <div className="broadcast-layout" aria-busy="true">
        <div className="broadcast-main">
          <div className="stage skeleton" />
          <p className="hint-text">불러오는 중…</p>
        </div>
      </div>
    )
  }
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
    <div className="broadcast-layout">
      <article className="broadcast-main">
        {isOwner && broadcast.status !== 'ENDED' ? (
          <Publisher broadcast={broadcast} onChanged={onChanged} />
        ) : (
          <Screen broadcast={broadcast} onLost={onChanged} />
        )}
        <div className="broadcast-info">
          <h1 className="broadcast-heading">{broadcast.title}</h1>
          <p className="broadcast-meta">
            <StatusBadge status={broadcast.status} /> {broadcast.ownerName}
            {' · '}
            <time dateTime={broadcast.createdAt}>{new Date(broadcast.createdAt).toLocaleString('ko-KR')}</time>
          </p>
          {broadcast.description && <p className="description">{broadcast.description}</p>}
          {isOwner && (
            <div className="actions">
              <Link to={`/broadcasts/${broadcast.id}/edit`} className="button">
                수정
              </Link>
              <button type="button" className="button-danger" onClick={onDelete} disabled={pending}>
                삭제
              </button>
            </div>
          )}
          {error && (
            <p className="error" role="alert">
              {error}
            </p>
          )}
        </div>
        {broadcast.status === 'ENDED' && <BroadcastRecordings key={broadcast.id} broadcastId={broadcast.id} />}
      </article>
      <aside className="broadcast-side">
        <DonationPanel
          broadcast={broadcast}
          state={donations.state}
          loadError={donations.loadError}
          onRetry={donations.retry}
        />
        <CommentPanel key={broadcast.id} broadcast={broadcast} onDonations={donations.dispatch} />
      </aside>
    </div>
  )
}

function Screen({ broadcast, onLost }: { broadcast: Broadcast; onLost: () => void }) {
  switch (broadcast.status) {
    case 'READY':
      return <StageNotice>아직 방송 전입니다.</StageNotice>
    case 'LIVE':
      return <Player key={broadcast.id} broadcastId={broadcast.id} onLost={onLost} />
    case 'INTERRUPTED':
      return <StageNotice warning>방송 연결이 끊겼습니다. 다시 연결되면 이어서 볼 수 있습니다.</StageNotice>
    case 'ENDED':
      return <StageNotice>종료된 방송입니다.</StageNotice>
  }
}

// Takes the player's place at the player's size, so the page does not jump when the broadcast changes state.
function StageNotice({ children, warning = false }: { children: string; warning?: boolean }) {
  return (
    <div className={warning ? 'stage stage-notice stage-warning' : 'stage stage-notice'}>
      <svg className="stage-icon" viewBox="0 0 24 24" aria-hidden="true">
        <rect x="2" y="5" width="15" height="14" rx="2" />
        <path d="m17 10 5-3v10l-5-3" />
      </svg>
      <p>{children}</p>
    </div>
  )
}
