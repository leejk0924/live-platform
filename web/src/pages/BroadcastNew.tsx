import { useState } from 'react'
import { useNavigate } from 'react-router'
import { broadcastApi } from '../api/broadcasts.ts'
import { errorMessage } from '../api/client.ts'
import type { BroadcastInput } from '../api/types.ts'
import RequireSignIn from '../auth/RequireSignIn.tsx'
import BroadcastForm from '../broadcasts/BroadcastForm.tsx'

export default function BroadcastNew() {
  return <RequireSignIn>{() => <CreateForm />}</RequireSignIn>
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
    <>
      <h1>방송 만들기</h1>
      <BroadcastForm submitLabel="만들기" pending={pending} error={error} onSubmit={onSubmit} />
    </>
  )
}
