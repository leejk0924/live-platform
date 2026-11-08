import assert from 'node:assert/strict'
import { test } from 'node:test'
import { ApiError } from '../api/client.ts'
import type { Comment } from '../api/types.ts'
import { draftAfterSaveError, saveArgs, startEdit } from './commentEdit.ts'

const comment = (version: number, body: string): Comment => ({
  id: '1',
  broadcastId: '1',
  authorId: '7',
  authorName: 'viewer',
  body,
  deleted: false,
  edited: version > 1,
  version,
  revision: String(version),
  createdAt: '2026-11-09T00:00:00Z',
})

test('an edit saves against the version it started from', () => {
  const draft = { ...startEdit(comment(1, 'first')), body: 'mine' }
  // Another tab edits the comment while this draft is open; the stream brings version 2.
  const latest = comment(2, 'theirs')
  assert.deepEqual(saveArgs(latest, draft), { commentId: '1', version: 1, body: 'mine' })
})

test('a version conflict closes the draft so the current comment shows', () => {
  const draft = { body: 'mine', version: 1 }
  const conflict = new ApiError(409, 'VERSION_CONFLICT', 'changed elsewhere')
  assert.equal(draftAfterSaveError(draft, conflict), null)
})

test('other save errors keep the draft for another try', () => {
  const draft = { body: 'mine', version: 1 }
  const errors = [new ApiError(429, 'RATE_LIMITED', 'slow down'), new ApiError(500, 'INTERNAL', 'x'), new Error('offline')]
  for (const error of errors) {
    assert.equal(draftAfterSaveError(draft, error), draft, error.message)
  }
})
