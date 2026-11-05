import { useEffect, useState } from 'react'

type ApiStatus = 'checking' | 'ok' | 'unreachable'

export default function App() {
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
    <main>
      <h1>live-platform</h1>
      <p className={`status status-${status}`}>API: {status}</p>
    </main>
  )
}
