import createClient from 'openapi-fetch'
import type { paths } from './schema.d.ts'

export class ApiError extends Error {
  readonly status: number
  readonly code: string
  readonly traceId: string | undefined

  constructor(status: number, code: string, message: string, traceId?: string) {
    super(message)
    this.name = 'ApiError'
    this.status = status
    this.code = code
    this.traceId = traceId
  }
}

export const api = createClient<paths>({ credentials: 'same-origin' })

api.use({
  onRequest({ request }) {
    // The server rejects state-changing requests without this header (CSRF guard).
    if (request.method !== 'GET') request.headers.set('X-Requested-With', 'live-platform')
  },
  onError() {
    return new ApiError(0, 'NETWORK_ERROR', '서버에 연결할 수 없습니다.')
  },
})

type Outcome<T> = { data?: T; error?: unknown; response: Response }

export async function unwrap<T>(pending: Promise<Outcome<T>>): Promise<T> {
  const { data, error, response } = await pending
  if (!response.ok) throw toApiError(response.status, error)
  return data as T
}

function toApiError(status: number, body: unknown): ApiError {
  if (isErrorBody(body)) return new ApiError(status, body.code, body.message, body.traceId)
  // A proxy error page (e.g. Caddy 502) has no envelope.
  return new ApiError(status, 'UNEXPECTED_RESPONSE', `요청에 실패했습니다. (${status})`)
}

function isErrorBody(body: unknown): body is { code: string; message: string; traceId?: string } {
  if (typeof body !== 'object' || body === null) return false
  const { code, message } = body as Record<string, unknown>
  return typeof code === 'string' && typeof message === 'string'
}

export function errorMessage(error: unknown): string {
  return error instanceof ApiError ? error.message : '알 수 없는 오류가 발생했습니다.'
}
