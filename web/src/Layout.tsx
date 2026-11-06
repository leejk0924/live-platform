import { useState } from 'react'
import { Link, Outlet } from 'react-router'
import { errorMessage } from './api/client.ts'
import { useAuth } from './auth/context.ts'

export default function Layout() {
  return (
    <>
      <header className="app-header">
        <Link to="/" className="app-name">
          live-platform
        </Link>
        <AccountMenu />
      </header>
      <main>
        <Outlet />
      </main>
    </>
  )
}

function AccountMenu() {
  const { auth, logout } = useAuth()
  const [error, setError] = useState<string | null>(null)
  const [pending, setPending] = useState(false)

  if (auth.status === 'loading') return null

  if (auth.status === 'signedOut') {
    return (
      <nav className="account">
        <Link to="/login">로그인</Link>
        <Link to="/signup">회원가입</Link>
      </nav>
    )
  }

  async function onLogout() {
    setPending(true)
    setError(null)
    try {
      await logout()
    } catch (e) {
      setError(errorMessage(e))
    } finally {
      setPending(false)
    }
  }

  return (
    <nav className="account">
      <span className="username">{auth.user.username}</span>
      <button type="button" onClick={onLogout} disabled={pending}>
        로그아웃
      </button>
      {error && (
        <span className="error" role="alert">
          {error}
        </span>
      )}
    </nav>
  )
}
