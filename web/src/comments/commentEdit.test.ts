import assert from 'node:assert/strict'
import { test } from 'node:test'
import type { Comment } from '../api/types.ts'
import { startEdit } from './commentEdit.ts'

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
  const draft = startEdit(comment(1, 'first'))
  // Another tab edits the comment while this draft is open; the stream brings version 2.
  const typed = { ...draft, body: 'mine' }
  assert.deepEqual(typed, { body: 'mine', version: 1 })
})
