import { Link, Navigate } from 'react-router'
import { authApi } from '../api/auth.ts'
import { useAuth } from '../auth/context.ts'
import CredentialsForm from '../auth/CredentialsForm.tsx'
import { useSignIn } from '../auth/useSignIn.ts'

export default function Signup() {
  const { auth } = useAuth()
  const { pending, error, run } = useSignIn()

  // A successful signup signs the user in, and this redirect takes them home.
  if (auth.status === 'signedIn') return <Navigate to="/" replace />

  return (
    <>
      <h1>회원가입</h1>
      <CredentialsForm
        mode="signup"
        pending={pending}
        error={error}
        onSubmit={(credentials) => run(() => authApi.signup(credentials))}
      />
      <p>
        이미 계정이 있으면 <Link to="/login">로그인</Link>
      </p>
    </>
  )
}
