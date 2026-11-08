import type { Comment } from '../api/types.ts'

// The version is taken when editing starts, not at save: saving against whatever version the
// stream brought meanwhile would silently overwrite an edit made in another tab.
export type EditDraft = { body: string; version: number }

export const startEdit = (comment: Comment): EditDraft => ({ body: comment.body ?? '', version: comment.version })
