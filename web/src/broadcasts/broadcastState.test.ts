import assert from 'node:assert/strict'
import { test } from 'node:test'
import type { Broadcast } from '../api/types.ts'
import { ApiError } from '../api/client.ts'
import { failed, loaded, reloading, type BroadcastState } from './broadcastState.ts'

const broadcast = { id: '1', status: 'LIVE' } as Broadcast
const live: BroadcastState = { status: 'loaded', broadcast, refreshFailed: false }

test('a failed first load is a failure', () => {
  const error = new Error('down')
  assert.deepEqual(failed({ status: 'loading' }, '1', error), { status: 'failed', error })
})

test('a failed refresh keeps the broadcast already shown', () => {
  const next = failed(live, '1', new Error('down'))
  assert.deepEqual(next, { status: 'loaded', broadcast, refreshFailed: true })
})

test('every failed refresh is a new state, so the next refresh gets scheduled', () => {
  const once = failed(live, '1', new Error('down'))
  assert.notEqual(failed(once, '1', new Error('down')), once)
})

test('a refresh that finds the broadcast gone shows it as gone', () => {
  const error = new ApiError(404, 'BROADCAST_NOT_FOUND', 'gone')
  assert.deepEqual(failed(live, '1', error), { status: 'failed', error })
})

test('a failure for another broadcast does not keep the old one', () => {
  const error = new Error('down')
  assert.deepEqual(failed(live, '2', error), { status: 'failed', error })
})

test('a successful refresh clears the failure', () => {
  assert.deepEqual(loaded(broadcast), live)
})

test('retrying a failed load shows loading', () => {
  assert.deepEqual(reloading({ status: 'failed', error: new Error('down') }), { status: 'loading' })
})

test('refreshing a loaded broadcast keeps it on screen', () => {
  assert.equal(reloading(live), live)
})
