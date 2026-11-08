import { Link } from 'react-router'
import { ApiError, errorMessage } from '../api/client.ts'

export default function LoadFailed({ error, onRetry }: { error: unknown; onRetry?: () => void }) {
  const notFound = error instanceof ApiError && error.code === 'BROADCAST_NOT_FOUND'
  return (
    <>
      <h1>{notFound ? '방송을 찾을 수 없습니다' : '방송을 불러오지 못했습니다'}</h1>
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
