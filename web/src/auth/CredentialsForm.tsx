import type { FormEvent } from 'react'
import type { Credentials } from '../api/types.ts'

type Props = {
  mode: 'login' | 'signup'
  pending: boolean
  error: string | null
  onSubmit: (credentials: Credentials) => void
}

export default function CredentialsForm({ mode, pending, error, onSubmit }: Props) {
  const signup = mode === 'signup'

  function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    const data = new FormData(event.currentTarget)
    onSubmit({ username: String(data.get('username')), password: String(data.get('password')) })
  }

  return (
    <form className="credentials" onSubmit={handleSubmit}>
      <label>
        아이디
        <input
          name="username"
          required
          autoComplete="username"
          autoCapitalize="none"
          spellCheck={false}
          {...(signup && {
            minLength: 3,
            maxLength: 20,
            pattern: '(?!demo-)[a-z0-9_\\-]+',
            title: '영문 소문자, 숫자, _, - 로 3~20자 (demo- 로 시작할 수 없음)',
            'aria-describedby': 'username-hint',
          })}
        />
      </label>
      {signup && (
        <small id="username-hint" className="hint">
          영문 소문자, 숫자, _, - 로 3~20자. demo- 로 시작할 수 없습니다.
        </small>
      )}
      <label>
        비밀번호
        <input
          name="password"
          type="password"
          required
          autoComplete={signup ? 'new-password' : 'current-password'}
          {...(signup && { minLength: 10, maxLength: 128, 'aria-describedby': 'password-hint' })}
        />
      </label>
      {signup && (
        <small id="password-hint" className="hint">
          10~128자. 널리 알려진 비밀번호는 쓸 수 없습니다.
        </small>
      )}
      {error && (
        <p className="error" role="alert">
          {error}
        </p>
      )}
      <button type="submit" disabled={pending}>
        {signup ? '회원가입' : '로그인'}
      </button>
    </form>
  )
}
