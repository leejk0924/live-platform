import type { ReactNode } from 'react'
import { useSearchParams } from 'react-router'
import { useAuth } from '../auth/context.ts'
import { broadcastUrl, parseBroadcastId } from './payUrls.ts'

// Frame for the /pay pages. Not RequireSignIn: its redirect to /login drops the query string and
// comes back by router navigation, which would keep the strict CSP and block Toss.
export default function PayPage({
  title,
  children,
}: {
  title: string
  children: (broadcastId: string, search: URLSearchParams) => ReactNode
}) {
  const { auth } = useAuth()
  const [search] = useSearchParams()
  const broadcastId = parseBroadcastId(search)

  return (
    <section className="page-card">
      <h1 className="page-title">{title}</h1>
      {broadcastId === null ? (
        <p className="error" role="alert">
          어느 방송의 후원인지 알 수 없습니다.
        </p>
      ) : auth.status === 'loading' ? (
        <p className="hint-text">불러오는 중…</p>
      ) : auth.status === 'signedOut' ? (
        <p className="error" role="alert">
          로그인이 필요합니다. 로그인한 뒤 방송 화면에서 다시 후원하세요.
        </p>
      ) : (
        children(broadcastId, search)
      )}
      <BackToBroadcast broadcastId={broadcastId} />
    </section>
  )
}

// A plain anchor, so the broadcast page loads with the strict CSP again.
function BackToBroadcast({ broadcastId }: { broadcastId: string | null }) {
  return (
    <p className="links">
      <a href={broadcastId === null ? '/' : broadcastUrl(broadcastId)}>
        {broadcastId === null ? '홈으로' : '방송으로 돌아가기'}
      </a>
    </p>
  )
}
