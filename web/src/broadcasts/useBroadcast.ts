import { useCallback, useEffect, useState } from 'react'
import { broadcastApi } from '../api/broadcasts.ts'
import type { Broadcast } from '../api/types.ts'

export type BroadcastState =
  | { status: 'loading' }
  | { status: 'failed'; error: unknown }
  | { status: 'loaded'; broadcast: Broadcast }

export function useBroadcast(id: string) {
  const [state, setState] = useState<BroadcastState>({ status: 'loading' })
  const [attempt, setAttempt] = useState(0)

  useEffect(() => {
    let active = true
    broadcastApi.get(id).then(
      (broadcast) => active && setState({ status: 'loaded', broadcast }),
      (error: unknown) => active && setState({ status: 'failed', error }),
    )
    return () => {
      active = false
    }
  }, [id, attempt])

  const reload = useCallback(() => setAttempt((n) => n + 1), [])

  return { state, reload }
}
