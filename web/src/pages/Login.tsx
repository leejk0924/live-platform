import { useEffect, useState } from 'react'
import { Link, Navigate } from 'react-router'
import { authApi } from '../api/auth.ts'
import { useAuth } from '../auth/context.ts'
import CredentialsForm from '../auth/CredentialsForm.tsx'
import { useSignIn } from '../auth/useSignIn.ts'

export default function Login() {
  const { auth } = useAuth()
  const { pending, error, run } = useSignIn()
  const [demoEnabled, setDemoEnabled] = useState(false)

  useEffect(() => {
    let active = true
    authApi
      .demoStatus()
      .then((status) => {
        if (active) setDemoEnabled(status.enabled)
      })
      .catch(() => {})
    return () => {
      active = false
    }
  }, [])

  // A successful sign-in lands here too, which is how the page leaves for home.
  if (auth.status === 'signedIn') return <Navigate to="/" replace />

  return (
    <>
      <h1>로그인</h1>
      <CredentialsForm
        mode="login"
        pending={pending}
        error={error}
        onSubmit={(credentials) => run(() => authApi.login(credentials))}
      />
      {demoEnabled && (
        <section className="demo">
          <h2>데모 계정</h2>
          <button type="button" disabled={pending} onClick={() => run(() => authApi.demoLogin('streamer'))}>
            데모 방송인으로 로그인
          </button>
          <button type="button" disabled={pending} onClick={() => run(() => authApi.demoLogin('viewer'))}>
            데모 시청자로 로그인
          </button>
        </section>
      )}
      <p>
        계정이 없으면 <Link to="/signup">회원가입</Link>
      </p>
    </>
  )
}
