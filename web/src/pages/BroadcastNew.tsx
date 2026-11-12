import { useState } from 'react'
import { Link, useNavigate } from 'react-router'
import { broadcastApi } from '../api/broadcasts.ts'
import { errorMessage } from '../api/client.ts'
import type { BroadcastInput } from '../api/types.ts'
import RequireSignIn from '../auth/RequireSignIn.tsx'
import { canCreateBroadcast } from '../auth/role.ts'
import BroadcastForm from '../broadcasts/BroadcastForm.tsx'

export default function BroadcastNew() {
  return (
    <RequireSignIn>
      {(user) =>
        canCreateBroadcast({ status: 'signedIn', user }) ? (
          <CreateForm />
        ) : (
          <section className="page-card">
            <h1>방송 만들기</h1>
            <p>방송인 계정만 방송을 만들 수 있습니다. 시청과 댓글, 후원은 지금 계정으로 할 수 있습니다.</p>
            <Link to="/" className="button">
              방송 목록으로
            </Link>
          </section>
        )
      }
    </RequireSignIn>
  )
}

function CreateForm() {
  const navigate = useNavigate()
  const [pending, setPending] = useState(false)
  const [error, setError] = useState<string | null>(null)

  async function onSubmit(input: BroadcastInput) {
    setPending(true)
    setError(null)
    try {
      const created = await broadcastApi.create(input)
      navigate(`/broadcasts/${created.id}`, { replace: true })
    } catch (e) {
      setError(errorMessage(e))
      setPending(false)
    }
  }

  return (
    <section className="page-card">
      <h1>방송 만들기</h1>
      <BroadcastForm submitLabel="만들기" pending={pending} error={error} onSubmit={onSubmit} />
    </section>
  )
}
