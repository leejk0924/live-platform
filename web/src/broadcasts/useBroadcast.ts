import { useCallback, useEffect, useState } from 'react'
import { broadcastApi } from '../api/broadcasts.ts'
import { failed, loaded, reloading, type BroadcastState } from './broadcastState.ts'

export function useBroadcast(id: string) {
  const [state, setState] = useState<BroadcastState>({ status: 'loading' })
  const [attempt, setAttempt] = useState(0)

  useEffect(() => {
    let active = true
    broadcastApi.get(id).then(
      (broadcast) => active && setState(loaded(broadcast)),
      (error: unknown) => active && setState((prev) => failed(prev, id, error)),
    )
    return () => {
      active = false
    }
  }, [id, attempt])

  const reload = useCallback(() => {
    setState(reloading)
    setAttempt((n) => n + 1)
  }, [])

  return { state, reload }
}
