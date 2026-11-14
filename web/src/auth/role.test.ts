import assert from 'node:assert/strict'
import { test } from 'node:test'
import type { User } from '../api/types.ts'
import { canCreateBroadcast } from './role.ts'

const user = (role: User['role']): User => ({ id: '1', username: 'someone', role })

const cases: { name: string; user: User | undefined; expected: boolean }[] = [
  { name: 'streamer', user: user('streamer'), expected: true },
  { name: 'viewer', user: user('viewer'), expected: false },
  { name: 'no user (signed out or loading)', user: undefined, expected: false },
]

for (const { name, user, expected } of cases) {
  test(`canCreateBroadcast: ${name} is ${expected}`, () => {
    assert.equal(canCreateBroadcast(user), expected)
  })
}
