import { useEffect, useState } from 'react'
import { Link } from 'react-router'
import { broadcastApi } from '../api/broadcasts.ts'
import { errorMessage } from '../api/client.ts'
import type { Broadcast } from '../api/types.ts'
import { useAuth } from '../auth/context.ts'
import { statusLabel } from '../broadcasts/status.ts'

type Feed = {
  items: Broadcast[]
  nextBefore: string | null
  loading: boolean
  error: string | null
}

export default function Home() {
  const { auth } = useAuth()

  return (
    <>
      <div className="page-title">
        <h1>방송</h1>
        {auth.status === 'signedIn' && (
          <Link to="/broadcasts/new" className="button">
            방송 만들기
          </Link>
        )}
      </div>
      <BroadcastFeed live title="방송 중" empty="지금 방송 중인 방송이 없습니다." />
      <BroadcastFeed live={false} title="지난·예정 방송" empty="지난 방송이나 예정된 방송이 없습니다." />
    </>
  )
}

function BroadcastFeed({ live, title, empty }: { live: boolean; title: string; empty: string }) {
  const [feed, setFeed] = useState<Feed>({ items: [], nextBefore: null, loading: true, error: null })

  useEffect(() => {
    let active = true
    broadcastApi.list({ live }).then(
      (page) => active && setFeed({ items: page.items, nextBefore: page.nextBefore, loading: false, error: null }),
      (e: unknown) => active && setFeed((f) => ({ ...f, loading: false, error: errorMessage(e) })),
    )
    return () => {
      active = false
    }
  }, [live])

  async function loadMore(before: string) {
    setFeed((f) => ({ ...f, loading: true, error: null }))
    try {
      const page = await broadcastApi.list({ live, before })
      setFeed((f) => ({ items: [...f.items, ...page.items], nextBefore: page.nextBefore, loading: false, error: null }))
    } catch (e) {
      setFeed((f) => ({ ...f, loading: false, error: errorMessage(e) }))
    }
  }

  const { nextBefore } = feed

  return (
    <section className="feed" aria-label={title}>
      <h2>{title}</h2>
      {feed.items.length > 0 && (
        <ul className="broadcast-list">
          {feed.items.map((b) => (
            <li key={b.id} className={`status-${b.status}`}>
              <Link to={`/broadcasts/${b.id}`}>
                <span className="broadcast-title">{b.title}</span>
                <span className="broadcast-meta">
                  {b.ownerName} · <span className={`badge badge-${b.status}`}>{statusLabel[b.status]}</span>
                </span>
              </Link>
            </li>
          ))}
        </ul>
      )}
      {!feed.loading && !feed.error && feed.items.length === 0 && <p>{empty}</p>}
      {feed.loading && <p>불러오는 중…</p>}
      {feed.error && (
        <p className="error" role="alert">
          {feed.error}
        </p>
      )}
      {nextBefore && !feed.loading && (
        <button type="button" onClick={() => loadMore(nextBefore)}>
          더 보기
        </button>
      )}
    </section>
  )
}
