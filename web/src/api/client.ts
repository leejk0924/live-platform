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

// Server messages are English (API contract); the UI shows Korean by code.
// NETWORK_ERROR and UNEXPECTED_RESPONSE are built here with Korean messages already.
const MESSAGES: Record<string, string> = {
  INVALID_CREDENTIALS: '아이디 또는 비밀번호가 올바르지 않습니다.',
  USERNAME_TAKEN: '이미 사용 중인 아이디입니다.',
  INVALID_USERNAME:
    '아이디는 3~20자의 영문 소문자, 숫자, _, - 만 쓸 수 있고 demo-로 시작할 수 없습니다.',
  PASSWORD_TOO_SHORT: '비밀번호는 10자 이상이어야 합니다.',
  PASSWORD_TOO_LONG: '비밀번호는 128자 이하여야 합니다.',
  PASSWORD_BREACHED: '유출된 적이 있는 비밀번호입니다. 다른 비밀번호를 쓰세요.',
  RATE_LIMITED: '요청이 너무 많습니다. 잠시 후 다시 시도하세요.',
  CSRF_REJECTED: '허용되지 않은 출처의 요청입니다. 페이지를 새로고침하세요.',
  UNAUTHENTICATED: '로그인이 필요합니다.',
  NOT_FOUND: '요청한 대상을 찾을 수 없습니다.',
  INVALID_REQUEST: '요청 형식이 올바르지 않습니다.',
  BROADCAST_NOT_FOUND: '방송을 찾을 수 없습니다.',
  NOT_BROADCAST_OWNER: '방송을 만든 사람만 바꿀 수 있습니다.',
  VERSION_CONFLICT: '다른 곳에서 방송이 수정되었습니다. 새로고침한 뒤 다시 시도하세요.',
  BROADCAST_NOT_DELETABLE: '방송 중이거나 중단된 방송은 삭제할 수 없습니다.',
  INVALID_TITLE: '제목은 1~100자여야 합니다.',
  INVALID_DESCRIPTION: '설명은 2000자 이하여야 합니다.',
  IF_MATCH_REQUIRED: '방송 버전 정보가 없습니다. 새로고침한 뒤 다시 시도하세요.',
  INVALID_IF_MATCH: '방송 버전 정보가 올바르지 않습니다. 새로고침한 뒤 다시 시도하세요.',
  INVALID_QUERY: '목록 요청 값이 올바르지 않습니다.',
  INVALID_PAGE_SIZE: '한 번에 1~50개까지 불러올 수 있습니다.',
  INTERNAL: '서버 오류가 발생했습니다. 잠시 후 다시 시도하세요.',
  METHOD_NOT_ALLOWED: '허용되지 않는 요청입니다.',
}

export function errorMessage(error: unknown): string {
  if (!(error instanceof ApiError)) return '알 수 없는 오류가 발생했습니다.'
  return MESSAGES[error.code] ?? error.message
}
