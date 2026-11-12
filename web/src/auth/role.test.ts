import assert from 'node:assert/strict'
import { test } from 'node:test'
import type { User } from '../api/types.ts'
import type { AuthState } from './context.ts'
import { canCreateBroadcast } from './role.ts'

const user = (role: User['role']): User => ({ id: '1', username: 'someone', role })

const cases: { name: string; auth: AuthState; expected: boolean }[] = [
  { name: 'streamer', auth: { status: 'signedIn', user: user('streamer') }, expected: true },
  { name: 'viewer', auth: { status: 'signedIn', user: user('viewer') }, expected: false },
  { name: 'signed out', auth: { status: 'signedOut' }, expected: false },
  { name: 'loading', auth: { status: 'loading' }, expected: false },
]

for (const { name, auth, expected } of cases) {
  test(`canCreateBroadcast: ${name} is ${expected}`, () => {
    assert.equal(canCreateBroadcast(auth), expected)
  })
}
