import { useState } from 'react'
import { errorMessage } from '../api/client.ts'
import type { User } from '../api/types.ts'
import { useAuth } from './context.ts'

export function useSignIn() {
  const { signedIn } = useAuth()
  const [pending, setPending] = useState(false)
  const [error, setError] = useState<string | null>(null)

  async function run(action: () => Promise<User>) {
    setPending(true)
    setError(null)
    try {
      signedIn(await action())
    } catch (e) {
      setError(errorMessage(e))
    } finally {
      setPending(false)
    }
  }

  return { pending, error, run }
}
