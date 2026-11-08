import assert from 'node:assert/strict'
import { test } from 'node:test'
import type { Comment } from '../api/types.ts'
import { commentsReducer, initialComments, MAX_KEPT, visibleComments, type CommentsState } from './commentsState.ts'

const comment = (id: string, revision: string, body: string | null = `c${id}`): Comment => ({
  id,
  broadcastId: '1',
  authorId: '7',
  authorName: 'viewer',
  body,
  deleted: body === null,
  edited: false,
  version: 1,
  revision,
  createdAt: '2026-11-09T00:00:00Z',
})

const loaded = (items: Comment[]): CommentsState =>
  commentsReducer(initialComments, { type: 'loaded', items, slowModeSeconds: 0 })
const changed = (state: CommentsState, ...items: Comment[]) => commentsReducer(state, { type: 'changed', items })
const shown = (state: CommentsState) => visibleComments(state).map((c) => `${c.id}:${c.body}`)

test('a page arrives newest first and shows oldest first', () => {
  assert.deepEqual(shown(loaded([comment('3', '3'), comment('2', '2'), comment('1', '1')])), ['1:c1', '2:c2', '3:c3'])
})

test('comments sort by numeric id, not by string', () => {
  assert.deepEqual(shown(loaded([comment('10', '2'), comment('9', '1')])), ['9:c9', '10:c10'])
})

test('a new comment lands at the bottom', () => {
  assert.deepEqual(shown(changed(loaded([comment('1', '1')]), comment('2', '2'))), ['1:c1', '2:c2'])
})

test('an older copy arriving after a newer one is ignored', () => {
  const state = changed(loaded([]), comment('1', '5', 'edited'), comment('1', '3', 'original'))
  assert.deepEqual(shown(state), ['1:edited'])
})

test('the same change delivered twice shows once', () => {
  const once = changed(loaded([]), comment('1', '1'))
  assert.deepEqual(shown(changed(once, comment('1', '1'))), ['1:c1'])
})

test('revisions compare as numbers', () => {
  const state = changed(loaded([comment('1', '9', 'old')]), comment('1', '10', 'new'))
  assert.deepEqual(shown(state), ['1:new'])
})

test('an edit then a delete removes the comment', () => {
  const edited = changed(loaded([comment('1', '1'), comment('2', '2')]), comment('1', '3', 'edited'))
  assert.deepEqual(shown(changed(edited, comment('1', '4', null))), ['2:c2'])
})

test('a stale edit after a delete does not bring the comment back', () => {
  const deleted = changed(loaded([comment('1', '1')]), comment('1', '4', null))
  assert.deepEqual(shown(changed(deleted, comment('1', '3', 'edited'))), [])
})

test('the own delete hides the comment before the stream confirms it', () => {
  const state = commentsReducer(loaded([comment('1', '1'), comment('2', '2')]), { type: 'deleted', id: '1' })
  assert.deepEqual(shown(state), ['2:c2'])
  assert.deepEqual(shown(changed(state, comment('1', '5', null))), ['2:c2'])
})

test('a reload replaces what was kept, dropping comments deleted meanwhile', () => {
  const before = changed(loaded([comment('1', '1')]), comment('2', '2'))
  const after = commentsReducer(before, { type: 'loaded', items: [comment('2', '2')], slowModeSeconds: 5 })
  assert.deepEqual(shown(after), ['2:c2'])
  assert.equal(after.slowModeSeconds, 5)
})

test(`only the newest ${MAX_KEPT} comments are kept`, () => {
  const many = Array.from({ length: MAX_KEPT + 20 }, (_, i) => comment(String(i + 1), String(i + 1)))
  const state = changed(loaded([]), ...many)
  const ids = visibleComments(state).map((c) => c.id)
  assert.equal(ids.length, MAX_KEPT)
  assert.equal(ids[0], '21')
  assert.equal(ids.at(-1), String(MAX_KEPT + 20))
})

test('a policy event sets the slow mode', () => {
  assert.equal(commentsReducer(loaded([]), { type: 'policy', slowModeSeconds: 30 }).slowModeSeconds, 30)
})
