import { useCallback, useEffect, useMemo, useState, type ReactNode } from 'react'
import { authApi } from '../api/auth.ts'
import type { User } from '../api/types.ts'
import { AuthContext, type AuthState } from './context.ts'

// Any failure (401, network, 5xx) reads as signed out: the UI must not claim a session it cannot confirm.
async function loadAuth(): Promise<AuthState> {
  try {
    return { status: 'signedIn', user: await authApi.me() }
  } catch {
    return { status: 'signedOut' }
  }
}

export function AuthProvider({ children }: { children: ReactNode }) {
  const [auth, setAuth] = useState<AuthState>({ status: 'loading' })

  useEffect(() => {
    let active = true
    loadAuth().then((next) => {
      if (active) setAuth(next)
    })
    return () => {
      active = false
    }
  }, [])

  const refresh = useCallback(async () => setAuth(await loadAuth()), [])

  const logout = useCallback(async () => {
    await authApi.logout()
    setAuth({ status: 'signedOut' })
  }, [])

  const signedIn = useCallback((user: User) => setAuth({ status: 'signedIn', user }), [])

  const value = useMemo(() => ({ auth, signedIn, refresh, logout }), [auth, signedIn, refresh, logout])

  return <AuthContext value={value}>{children}</AuthContext>
}
