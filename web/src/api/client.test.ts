import assert from 'node:assert/strict'
import { test } from 'node:test'
import { ApiError, unwrap } from './client.ts'
import { eventsUrl } from './comments.ts'

const envelope = { code: 'RATE_LIMITED', message: 'slow down', traceId: 't1' }

async function failure(status: number, headers: Record<string, string>, error: unknown = envelope): Promise<ApiError> {
  const response = new Response(null, { status, headers })
  try {
    await unwrap(Promise.resolve({ error, response }))
  } catch (e) {
    assert.ok(e instanceof ApiError)
    return e
  }
  assert.fail('expected a rejection')
}

test('a 429 carries Retry-After in seconds', async () => {
  const error = await failure(429, { 'Retry-After': '3' })
  assert.equal(error.status, 429)
  assert.equal(error.code, 'RATE_LIMITED')
  assert.equal(error.traceId, 't1')
  assert.equal(error.retryAfter, 3)
})

test('a missing or HTTP-date Retry-After gives no wait', async () => {
  assert.equal((await failure(429, {})).retryAfter, undefined)
  assert.equal((await failure(429, { 'Retry-After': 'Wed, 21 Oct 2026 07:28:00 GMT' })).retryAfter, undefined)
})

test('an answer without the error envelope is an unexpected response', async () => {
  const error = await failure(502, {}, '<html>Bad Gateway</html>')
  assert.equal(error.code, 'UNEXPECTED_RESPONSE')
  assert.equal(error.status, 502)
})

test('the event stream resumes after the given revision', () => {
  assert.equal(eventsUrl('12', '0'), '/api/broadcasts/12/events?after=0')
  assert.equal(eventsUrl('12', '345'), '/api/broadcasts/12/events?after=345')
})
