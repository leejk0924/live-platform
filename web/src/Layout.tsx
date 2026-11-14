import { useState } from 'react'
import { Link, NavLink, Outlet } from 'react-router'
import { errorMessage } from './api/client.ts'
import { useAuth } from './auth/context.ts'
import { canCreateBroadcast } from './auth/role.ts'

export default function Layout() {
  return (
    <>
      <header className="app-header">
        <div className="app-header-inner">
          <Link to="/" className="app-name">
            <svg className="app-mark" viewBox="0 0 24 24" aria-hidden="true">
              <circle cx="12" cy="12" r="3.5" />
              <path d="M7 7a7 7 0 0 0 0 10M17 7a7 7 0 0 1 0 10" />
            </svg>
            live-platform
          </Link>
          <nav className="site-nav">
            <NavLink to="/recordings" className="nav-link">
              다시보기
            </NavLink>
          </nav>
          <AccountMenu />
        </div>
      </header>
      <main className="app-main">
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
        <Link to="/login" className="nav-link">
          로그인
        </Link>
        <Link to="/signup" className="button">
          회원가입
        </Link>
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
      {canCreateBroadcast(auth.user) && (
        <Link to="/broadcasts/new" className="button button-primary">
          방송하기
        </Link>
      )}
      <span className="username">{auth.user.username}</span>
      <button type="button" className="button-quiet" onClick={onLogout} disabled={pending}>
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
