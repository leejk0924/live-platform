import assert from 'node:assert/strict'
import { test } from 'node:test'
import { parseComments, parsePolicy } from './events.ts'

test('a comments event is an array of comments with decimal ids and revisions', () => {
  const comment = { id: '1', revision: '5', deleted: false, body: 'hi' }
  assert.deepEqual(parseComments(JSON.stringify([comment])), [comment])
  assert.deepEqual(parseComments('[]'), [])
})

test('anything else is refused', () => {
  const comment = { id: '1', revision: '5', deleted: false }
  const malformed = [
    { ...comment, id: 1 },
    { ...comment, id: '' },
    { ...comment, revision: '6.0' },
    { ...comment, revision: '-1' },
    { ...comment, revision: '1e3' },
    { ...comment, deleted: 'false' },
    { id: '1' },
  ]
  for (const data of ['', 'not json', '{}', 'null', '[null]', '[1]', ...malformed.map((c) => JSON.stringify([c]))]) {
    assert.equal(parseComments(data), null, data)
  }
})

test('a policy event is a slow mode of 0 to 300 whole seconds', () => {
  for (const slowModeSeconds of [0, 30, 300]) {
    assert.deepEqual(parsePolicy(JSON.stringify({ slowModeSeconds })), { slowModeSeconds })
  }
  for (const data of ['', 'null', '{}', '{"slowModeSeconds": -1}', '{"slowModeSeconds": 301}', '{"slowModeSeconds": 1.5}', '{"slowModeSeconds": "30"}']) {
    assert.equal(parsePolicy(data), null, data)
  }
})
