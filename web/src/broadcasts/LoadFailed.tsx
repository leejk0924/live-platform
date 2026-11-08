import { Link } from 'react-router'
import { ApiError, errorMessage } from '../api/client.ts'

const subjects = {
  broadcast: { notFoundCode: 'BROADCAST_NOT_FOUND', notFound: '방송을 찾을 수 없습니다', failed: '방송을 불러오지 못했습니다' },
  recording: { notFoundCode: 'RECORDING_NOT_FOUND', notFound: '녹화를 찾을 수 없습니다', failed: '녹화를 불러오지 못했습니다' },
}

export default function LoadFailed({
  error,
  onRetry,
  subject = 'broadcast',
}: {
  error: unknown
  onRetry?: () => void
  subject?: keyof typeof subjects
}) {
  const text = subjects[subject]
  const notFound = error instanceof ApiError && error.code === text.notFoundCode
  return (
    <>
      <h1>{notFound ? text.notFound : text.failed}</h1>
      {!notFound && (
        <p className="error" role="alert">
          {errorMessage(error)}
        </p>
      )}
      {!notFound && onRetry && (
        <div className="actions">
          <button type="button" onClick={onRetry}>
            다시 시도
          </button>
        </div>
      )}
      <p>
        <Link to="/">홈으로</Link>
      </p>
    </>
  )
}
