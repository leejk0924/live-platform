import type { Comment } from '../api/types.ts'

// The `data` of a `comments` event, or null when it is not what the server sends. The reducer
// orders and dedupes by `id` and `revision`, so those are the fields checked.
export function parseComments(data: string): Comment[] | null {
  let parsed: unknown
  try {
    parsed = JSON.parse(data)
  } catch {
    return null
  }
  return Array.isArray(parsed) && parsed.every(isComment) ? parsed : null
}

function isComment(value: unknown): value is Comment {
  if (typeof value !== 'object' || value === null) return false
  const { id, revision } = value as Record<string, unknown>
  return typeof id === 'string' && typeof revision === 'string'
}
