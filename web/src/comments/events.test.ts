import assert from 'node:assert/strict'
import { test } from 'node:test'
import { parseComments } from './events.ts'

test('a comments event is an array of comments with string ids and revisions', () => {
  const data = JSON.stringify([{ id: '1', revision: '5', body: 'hi' }])
  assert.deepEqual(parseComments(data), [{ id: '1', revision: '5', body: 'hi' }])
  assert.deepEqual(parseComments('[]'), [])
})

test('anything else is refused', () => {
  for (const data of ['', 'not json', '{}', 'null', '[null]', '[1]', '[{"id": 1, "revision": "5"}]', '[{"id": "1"}]']) {
    assert.equal(parseComments(data), null, data)
  }
})
