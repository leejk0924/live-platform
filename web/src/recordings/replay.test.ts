import assert from 'node:assert/strict'
import { test } from 'node:test'
import { formatDuration, formatSize, initialReplay, replayReducer, type ReplayEvent, type ReplayState } from './replay.ts'

const run = (events: ReplayEvent[], from: ReplayState = initialReplay) => events.reduce(replayReducer, from)
const at = { time: 42, playing: true }

test('a fetched URL is played', () => {
  assert.deepEqual(run([{ type: 'urlLoaded', url: '/a' }]), { kind: 'ready', url: '/a', resume: null, retried: false })
})

test('a media error asks for a new URL and remembers where playback was', () => {
  const state = run([{ type: 'urlLoaded', url: '/a' }, { type: 'progressed', time: 42 }, { type: 'mediaError', at }])
  assert.deepEqual(state, { kind: 'loading', resume: at, retried: true })
})

test('the new URL resumes from that position', () => {
  const state = run([{ type: 'urlLoaded', url: '/a' }, { type: 'mediaError', at }, { type: 'urlLoaded', url: '/b' }])
  assert.deepEqual(state, { kind: 'ready', url: '/b', resume: at, retried: true })
})

test('a second failure in a row gives up, keeping the first position', () => {
  const state = run([
    { type: 'urlLoaded', url: '/a' },
    { type: 'mediaError', at },
    { type: 'urlLoaded', url: '/b' },
    { type: 'mediaError', at: { time: 0, playing: false } },
  ])
  assert.deepEqual(state, { kind: 'failed', resume: at, error: null })
})

test('a fresh URL that loads but fails at the same spot gives up', () => {
  const state = run([
    { type: 'urlLoaded', url: '/a' },
    { type: 'progressed', time: 42 },
    { type: 'mediaError', at },
    { type: 'urlLoaded', url: '/b' },
    { type: 'progressed', time: 42.5 },
    { type: 'mediaError', at },
  ])
  assert.deepEqual(state, { kind: 'failed', resume: at, error: null })
})

test('playing on past the restored position resets the count, so a later expiry refreshes again', () => {
  const later = { time: 300, playing: true }
  const state = run([
    { type: 'urlLoaded', url: '/a' },
    { type: 'mediaError', at },
    { type: 'urlLoaded', url: '/b' },
    { type: 'progressed', time: 44 },
    { type: 'mediaError', at: later },
  ])
  assert.deepEqual(state, { kind: 'loading', resume: later, retried: true })
})

test('after a manual retry, playing on drops the old position', () => {
  const failed: ReplayState = { kind: 'failed', resume: at, error: null }
  const state = run([{ type: 'retry' }, { type: 'urlLoaded', url: '/b' }, { type: 'progressed', time: 60 }], failed)
  assert.deepEqual(state, { kind: 'ready', url: '/b', resume: null, retried: false })
})

test('failing to get a URL shows the API error', () => {
  const error = new Error('not ready')
  assert.deepEqual(run([{ type: 'urlFailed', error }]), { kind: 'failed', resume: null, error })
})

test('retrying after giving up starts over from the kept position', () => {
  const failed: ReplayState = { kind: 'failed', resume: at, error: null }
  assert.deepEqual(run([{ type: 'retry' }], failed), { kind: 'loading', resume: at, retried: false })
})

test('a media error while a URL is being fetched is ignored', () => {
  const loading: ReplayState = { kind: 'loading', resume: at, retried: true }
  assert.equal(run([{ type: 'mediaError', at: { time: 0, playing: false } }], loading), loading)
})

const durations = [
  { ms: 0, expected: '00:00' },
  { ms: 999, expected: '00:00' },
  { ms: 65_000, expected: '01:05' },
  { ms: 3_599_000, expected: '59:59' },
  { ms: 3_600_000, expected: '1:00:00' },
  { ms: 37_230_500, expected: '10:20:30' },
]

for (const { ms, expected } of durations) {
  test(`${ms} ms reads as ${expected}`, () => assert.equal(formatDuration(ms), expected))
}

test('sizes read in megabytes', () => {
  assert.equal(formatSize(12_345_678), '12.3 MB')
  assert.equal(formatSize(0), '0.0 MB')
})
