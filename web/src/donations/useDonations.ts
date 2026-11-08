import { useEffect, useReducer, useRef, useState } from 'react'
import { donationApi } from '../api/donations.ts'
import { donationsReducer, initialDonations, needsReload } from './donationsState.ts'

const FRESH_MS = 6000

// The list comes from the API; live changes come from the comment stream, which the caller
// forwards to `dispatch`.
export function useDonations(broadcastId: string) {
  const [state, dispatch] = useReducer(donationsReducer, initialDonations)
  const [loadError, setLoadError] = useState<unknown>(null)
  const [attempt, setAttempt] = useState(0)
  // The count a reload was last asked for, so a list the server keeps short cannot loop.
  const reloadedFor = useRef(0)
  const stale = needsReload(state)

  useEffect(() => {
    let active = true
    donationApi.list(broadcastId).then(
      (page) => {
        if (!active) return
        dispatch({ type: 'loaded', page })
        setLoadError(null)
      },
      (error: unknown) => active && setLoadError(error),
    )
    return () => {
      active = false
    }
  }, [broadcastId, attempt])

  useEffect(() => {
    if (!stale || state.count <= reloadedFor.current) return
    reloadedFor.current = state.count
    setAttempt((n) => n + 1)
  }, [stale, state.count])

  const freshId = state.fresh?.id
  useEffect(() => {
    if (freshId === undefined) return
    const timer = window.setTimeout(() => dispatch({ type: 'faded', id: freshId }), FRESH_MS)
    return () => window.clearTimeout(timer)
  }, [freshId])

  return { state, dispatch, loadError, retry: () => setAttempt((n) => n + 1) }
}
