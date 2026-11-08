import type { ChatPolicy, Comment } from '../api/types.ts'

const DECIMAL = /^\d+$/
// The server's longest slow mode.
const MAX_SLOW_MODE_SECONDS = 300

// The `data` of a `comments` event, or null when it is not what the server sends. The reducer
// orders and dedupes by `id` and `revision` as BigInt and drops `deleted` ones, so those are the
// fields checked.
export function parseComments(data: string): Comment[] | null {
  const parsed = parseJson(data)
  return Array.isArray(parsed) && parsed.every(isComment) ? parsed : null
}

// The `data` of a `policy` event, or null when it is not what the server sends.
export function parsePolicy(data: string): ChatPolicy | null {
  const parsed = parseJson(data)
  if (typeof parsed !== 'object' || parsed === null) return null
  const { slowModeSeconds: seconds } = parsed as Record<string, unknown>
  const valid = typeof seconds === 'number' && Number.isInteger(seconds) && seconds >= 0 && seconds <= MAX_SLOW_MODE_SECONDS
  return valid ? { slowModeSeconds: seconds } : null
}

// `undefined` for anything that is not JSON, which no JSON text parses to.
function parseJson(data: string): unknown {
  try {
    return JSON.parse(data) as unknown
  } catch {
    return undefined
  }
}

function isComment(value: unknown): value is Comment {
  if (typeof value !== 'object' || value === null) return false
  const { id, revision, deleted } = value as Record<string, unknown>
  return isDecimal(id) && isDecimal(revision) && typeof deleted === 'boolean'
}

function isDecimal(value: unknown): value is string {
  return typeof value === 'string' && DECIMAL.test(value)
}
