import type { ErrorBody } from './types.ts'

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

type Method = 'GET' | 'POST' | 'PUT' | 'PATCH' | 'DELETE'

export async function request<T>(method: Method, path: string, body?: unknown): Promise<T> {
  const headers: Record<string, string> = { Accept: 'application/json' }
  // The server rejects state-changing requests without this header (CSRF guard).
  if (method !== 'GET') headers['X-Requested-With'] = 'live-platform'
  if (body !== undefined) headers['Content-Type'] = 'application/json'

  let res: Response
  try {
    res = await fetch(path, {
      method,
      headers,
      credentials: 'same-origin',
      body: body === undefined ? undefined : JSON.stringify(body),
    })
  } catch {
    throw new ApiError(0, 'NETWORK_ERROR', '서버에 연결할 수 없습니다.')
  }

  if (!res.ok) throw await toApiError(res)
  if (res.status === 204) return undefined as T
  return (await res.json()) as T
}

async function toApiError(res: Response): Promise<ApiError> {
  const body = (await res.json().catch(() => null)) as Partial<ErrorBody> | null
  if (body && typeof body.code === 'string' && typeof body.message === 'string') {
    return new ApiError(res.status, body.code, body.message, body.traceId)
  }
  // A proxy error page (e.g. Caddy 502) has no envelope.
  return new ApiError(res.status, 'UNEXPECTED_RESPONSE', `요청에 실패했습니다. (${res.status})`)
}

export function errorMessage(error: unknown): string {
  return error instanceof ApiError ? error.message : '알 수 없는 오류가 발생했습니다.'
}
