import type { ReactNode } from 'react'
import { Navigate, useLocation } from 'react-router'
import type { User } from '../api/types.ts'
import { useAuth } from '../auth/context.ts'

export default function RequireSignIn({ children }: { children: (user: User) => ReactNode }) {
  const { auth } = useAuth()
  const location = useLocation()

  if (auth.status === 'loading') return null
  if (auth.status === 'signedOut') return <Navigate to="/login" replace state={{ from: location.pathname }} />
  return children(auth.user)
}
