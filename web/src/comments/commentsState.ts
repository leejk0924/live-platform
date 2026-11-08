import type { Comment } from '../api/types.ts'

// A long live chat would otherwise grow without bound; the oldest fall off the top.
export const MAX_KEPT = 500
// Deleted comments remembered so a late copy cannot bring them back; the oldest are forgotten first.
export const MAX_TOMBSTONES = 500

// `comments` holds the shown comments, ascending by id (Snowflake, so oldest first).
// `tombstones` maps a deleted comment's id to the revision it was deleted at: a copy that arrives
// late with a lower revision must not bring it back. Kept apart so deletions take no room from
// the comments shown.
export type CommentsState = {
  comments: readonly Comment[]
  tombstones: ReadonlyMap<string, string>
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

export const initialComments: CommentsState = { comments: [], tombstones: new Map(), slowModeSeconds: 0 }

export function commentsReducer(state: CommentsState, action: CommentsAction): CommentsState {
  switch (action.type) {
    case 'loaded':
      return { ...apply(initialComments, action.items), slowModeSeconds: action.slowModeSeconds }
    case 'changed':
      return apply(state, action.items)
    case 'deleted':
      return deleteNow(state, action.id)
    case 'policy':
      return { ...state, slowModeSeconds: action.slowModeSeconds }
  }
}

export const visibleComments = (state: CommentsState): readonly Comment[] => state.comments

// Applies each change unless the comment is already known at the same or a higher revision.
function apply(state: CommentsState, incoming: readonly Comment[]): CommentsState {
  const byId = new Map(state.comments.map((c) => [c.id, c]))
  const tombstones = new Map(state.tombstones)
  for (const comment of incoming) {
    const known = byId.get(comment.id)?.revision ?? tombstones.get(comment.id)
    if (known !== undefined && BigInt(comment.revision) <= BigInt(known)) continue
    if (comment.deleted) {
      byId.delete(comment.id)
      bury(tombstones, comment.id, comment.revision)
    } else {
      byId.set(comment.id, comment)
    }
  }
  return { ...state, comments: [...byId.values()].sort(byIdAscending).slice(-MAX_KEPT), tombstones }
}

// The delete's own revision is unknown until the stream's copy arrives; until then the comment's
// last revision keeps older copies out.
function deleteNow(state: CommentsState, id: string): CommentsState {
  const gone = state.comments.find((c) => c.id === id)
  if (!gone) return state
  const tombstones = new Map(state.tombstones)
  bury(tombstones, id, gone.revision)
  return { ...state, comments: state.comments.filter((c) => c.id !== id), tombstones }
}

function bury(tombstones: Map<string, string>, id: string, revision: string) {
  tombstones.delete(id)
  tombstones.set(id, revision)
  for (const oldest of tombstones.keys()) {
    if (tombstones.size <= MAX_TOMBSTONES) break
    tombstones.delete(oldest)
  }
}

const byIdAscending = (a: Comment, b: Comment): number => {
  const diff = BigInt(a.id) - BigInt(b.id)
  return diff < 0n ? -1 : diff > 0n ? 1 : 0
}
