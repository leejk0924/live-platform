import { useState } from 'react'
import { Link, useNavigate, useParams } from 'react-router'
import { broadcastApi } from '../api/broadcasts.ts'
import { ApiError, errorMessage } from '../api/client.ts'
import type { Broadcast, BroadcastInput, User } from '../api/types.ts'
import RequireSignIn from '../auth/RequireSignIn.tsx'
import BroadcastForm from '../broadcasts/BroadcastForm.tsx'
import LoadFailed from '../broadcasts/LoadFailed.tsx'
import { useBroadcast } from '../broadcasts/useBroadcast.ts'

export default function BroadcastEdit() {
  return <RequireSignIn>{(user) => <EditLoader user={user} />}</RequireSignIn>
}

function EditLoader({ user }: { user: User }) {
  const { id = '' } = useParams()
  const { state, reload } = useBroadcast(id)
  const [conflicted, setConflicted] = useState(false)

  if (state.status === 'loading') return <p>불러오는 중…</p>
  if (state.status === 'failed') return <LoadFailed error={state.error} />

  const broadcast = state.broadcast
  if (broadcast.ownerId !== user.id) {
    return (
      <>
        <h1>수정할 수 없습니다</h1>
        <p>본인이 만든 방송만 수정할 수 있습니다.</p>
        <p>
          <Link to={`/broadcasts/${broadcast.id}`}>방송으로 돌아가기</Link>
        </p>
      </>
    )
  }

  return (
    <>
      <h1>방송 수정</h1>
      {conflicted && (
        <p className="notice" role="status">
          다른 곳에서 수정되었습니다. 최신 내용을 불러왔으니 다시 수정해 주세요.
        </p>
      )}
      <EditForm
        // The reload after a conflict brings a new version; remounting resets the fields to the stored values.
        key={broadcast.version}
        broadcast={broadcast}
        onConflict={() => {
          setConflicted(true)
          reload()
        }}
      />
    </>
  )
}

function EditForm({ broadcast, onConflict }: { broadcast: Broadcast; onConflict: () => void }) {
  const navigate = useNavigate()
  const [pending, setPending] = useState(false)
  const [error, setError] = useState<string | null>(null)

  async function onSubmit(input: BroadcastInput) {
    setPending(true)
    setError(null)
    try {
      await broadcastApi.update(broadcast.id, broadcast.version, input)
      navigate(`/broadcasts/${broadcast.id}`, { replace: true })
    } catch (e) {
      setPending(false)
      if (e instanceof ApiError && e.code === 'VERSION_CONFLICT') onConflict()
      else setError(errorMessage(e))
    }
  }

  return (
    <>
      <BroadcastForm initial={broadcast} submitLabel="저장" pending={pending} error={error} onSubmit={onSubmit} />
      <p>
        <Link to={`/broadcasts/${broadcast.id}`}>취소</Link>
      </p>
    </>
  )
}
