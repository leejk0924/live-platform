import type { Comment } from '../api/types.ts'

// A long live chat would otherwise grow without bound; the oldest fall off the top.
export const MAX_KEPT = 500

// `comments` is ascending by id (Snowflake, so oldest first) and keeps deleted comments as
// tombstones: a copy that arrives late with a lower revision must not bring one back.
export type CommentsState = {
  comments: readonly Comment[]
  slowModeSeconds: number
}

export type CommentsAction =
  // A fresh list replaces everything: the list leaves deleted comments out, so after a reset
  // merging would keep comments deleted while the stream was away.
  | { type: 'loaded'; items: readonly Comment[]; slowModeSeconds: number }
  // A `comments` event, or the server's answer to this user's post or edit.
  | { type: 'changed'; items: readonly Comment[] }
  // This user's delete. The answer has no body, and the stream's copy follows.
  | { type: 'deleted'; id: string }
  | { type: 'policy'; slowModeSeconds: number }

export const initialComments: CommentsState = { comments: [], slowModeSeconds: 0 }

export function commentsReducer(state: CommentsState, action: CommentsAction): CommentsState {
  switch (action.type) {
    case 'loaded':
      return { comments: upsert([], action.items), slowModeSeconds: action.slowModeSeconds }
    case 'changed':
      return { ...state, comments: upsert(state.comments, action.items) }
    case 'deleted':
      return {
        ...state,
        comments: state.comments.map((c) => (c.id === action.id ? { ...c, body: null, deleted: true } : c)),
      }
    case 'policy':
      return { ...state, slowModeSeconds: action.slowModeSeconds }
  }
}

export const visibleComments = (state: CommentsState): Comment[] => state.comments.filter((c) => !c.deleted)

function upsert(kept: readonly Comment[], incoming: readonly Comment[]): Comment[] {
  const byId = new Map(kept.map((c) => [c.id, c]))
  for (const comment of incoming) {
    const current = byId.get(comment.id)
    if (!current || BigInt(comment.revision) > BigInt(current.revision)) byId.set(comment.id, comment)
  }
  return [...byId.values()].sort(byIdAscending).slice(-MAX_KEPT)
}

const byIdAscending = (a: Comment, b: Comment): number => {
  const diff = BigInt(a.id) - BigInt(b.id)
  return diff < 0n ? -1 : diff > 0n ? 1 : 0
}
