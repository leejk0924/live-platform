import assert from 'node:assert/strict'
import { test } from 'node:test'
import { backgroundAction } from './phase.ts'

const cases = [
  { name: 'hiding a phone mid-broadcast pauses', phase: 'publishing', hidden: true, pauses: true, status: 'LIVE', expected: 'pause' },
  { name: 'coming back within the grace resumes', phase: 'paused', hidden: false, pauses: true, status: 'INTERRUPTED', expected: 'resume' },
  { name: 'coming back after the broadcast ended goes idle', phase: 'paused', hidden: false, pauses: true, status: 'ENDED', expected: 'idle' },
  { name: 'staying hidden while paused does nothing', phase: 'paused', hidden: true, pauses: true, status: 'INTERRUPTED', expected: null },
  { name: 'a desktop keeps publishing in the background', phase: 'publishing', hidden: true, pauses: false, status: 'LIVE', expected: null },
  { name: 'hiding while idle does nothing', phase: 'idle', hidden: true, pauses: true, status: 'READY', expected: null },
  { name: 'hiding while connecting waits for publishing', phase: 'connecting', hidden: true, pauses: true, status: 'READY', expected: null },
  { name: 'showing while idle does nothing', phase: 'idle', hidden: false, pauses: true, status: 'INTERRUPTED', expected: null },
] as const

for (const { name, expected, ...input } of cases) {
  test(`backgroundAction: ${name}`, () => {
    assert.equal(backgroundAction(input), expected)
  })
}
