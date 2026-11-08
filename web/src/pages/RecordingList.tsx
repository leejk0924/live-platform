import { useEffect, useState } from 'react'
import { errorMessage } from '../api/client.ts'
import { recordingApi } from '../api/recordings.ts'
import type { Recording } from '../api/types.ts'
import RecordingItem from '../recordings/RecordingItem.tsx'

type Feed = {
  items: Recording[]
  nextBefore: string | null
  loading: boolean
  error: string | null
}

export default function RecordingList() {
  const [feed, setFeed] = useState<Feed>({ items: [], nextBefore: null, loading: true, error: null })

  useEffect(() => {
    let active = true
    recordingApi.list({}).then(
      (page) => active && setFeed({ items: page.items, nextBefore: page.nextBefore, loading: false, error: null }),
      (e: unknown) => active && setFeed((f) => ({ ...f, loading: false, error: errorMessage(e) })),
    )
    return () => {
      active = false
    }
  }, [])

  async function loadMore(before: string) {
    setFeed((f) => ({ ...f, loading: true, error: null }))
    try {
      const page = await recordingApi.list({ before })
      setFeed((f) => ({ items: [...f.items, ...page.items], nextBefore: page.nextBefore, loading: false, error: null }))
    } catch (e) {
      setFeed((f) => ({ ...f, loading: false, error: errorMessage(e) }))
    }
  }

  const { nextBefore } = feed

  return (
    <>
      <h1>다시보기</h1>
      <section className="feed" aria-label="녹화 목록">
        {feed.items.length > 0 && (
          <ul className="broadcast-list">
            {feed.items.map((r) => (
              <RecordingItem key={r.id} recording={r} withTitle />
            ))}
          </ul>
        )}
        {!feed.loading && !feed.error && feed.items.length === 0 && <p>아직 녹화된 방송이 없습니다.</p>}
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
    </>
  )
}
