import { ApiError } from '../api/client.ts'
import type { Comment } from '../api/types.ts'

// The version is taken when editing starts, not at save: saving against whatever version the
// stream brought meanwhile would silently overwrite an edit made in another tab.
export type EditDraft = { body: string; version: number }

export const startEdit = (comment: Comment): EditDraft => ({ body: comment.body ?? '', version: comment.version })

// What saving `draft` sends: the comment is the one shown now, the version the draft's own.
export const saveArgs = (comment: Comment, draft: EditDraft) => ({
  commentId: comment.id,
  version: draft.version,
  body: draft.body,
})

// The draft to keep after saving failed. A version conflict closes it: its version can never
// succeed again, and the form would hide the comment that changed. Editing again starts from the
// current version.
export function draftAfterSaveError(draft: EditDraft, error: unknown): EditDraft | null {
  return error instanceof ApiError && error.code === 'VERSION_CONFLICT' ? null : draft
}
