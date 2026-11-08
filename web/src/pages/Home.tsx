import { useEffect, useState } from 'react'
import { useAuth } from '../auth/context.ts'

type ApiStatus = 'checking' | 'ok' | 'unreachable'

export default function Home() {
  const { auth } = useAuth()
  const [status, setStatus] = useState<ApiStatus>('checking')

  useEffect(() => {
    const controller = new AbortController()
    fetch('/api/healthz', { signal: controller.signal })
      .then((res) => setStatus(res.ok ? 'ok' : 'unreachable'))
      .catch(() => {
        if (!controller.signal.aborted) setStatus('unreachable')
      })
    return () => controller.abort()
  }, [])

  return (
    <>
      <h1>live-platform</h1>
      <p className={`status status-${status}`}>API: {status}</p>
      {auth.status === 'signedIn' && <p>{auth.user.username}님으로 로그인했습니다.</p>}
      {auth.status === 'signedOut' && <p>로그인하지 않았습니다.</p>}
    </>
  )
}
