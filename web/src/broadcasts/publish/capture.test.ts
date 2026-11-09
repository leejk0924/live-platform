import assert from 'node:assert/strict'
import { test } from 'node:test'
import {
  availableSources,
  cameraVideoConstraints,
  otherFacing,
  pausesInBackground,
  pickOtherCamera,
} from './capture.ts'

const desktop = { getDisplayMedia: () => {}, getUserMedia: () => {} }
const phone = { getUserMedia: () => {} }

test('a desktop offers the screen first, then the camera', () => {
  assert.deepEqual(availableSources(desktop), ['screen', 'camera'])
})

test('a phone offers only the camera', () => {
  assert.deepEqual(availableSources(phone), ['camera'])
})

test('an insecure page has no media devices and offers nothing', () => {
  assert.deepEqual(availableSources(undefined), [])
  assert.deepEqual(availableSources({}), [])
})

test('only camera publishing on a mobile browser pauses in the background', () => {
  assert.equal(pausesInBackground('camera', phone), true)
  assert.equal(pausesInBackground('camera', desktop), false)
  assert.equal(pausesInBackground('screen', desktop), false)
})

test('otherFacing flips between front and back', () => {
  assert.equal(otherFacing('user'), 'environment')
  assert.equal(otherFacing('environment'), 'user')
})

test('camera constraints demand the facing and ask for 720p30', () => {
  assert.deepEqual(cameraVideoConstraints('environment'), {
    facingMode: { exact: 'environment' },
    width: { ideal: 1280 },
    height: { ideal: 720 },
    frameRate: { ideal: 30, max: 30 },
  })
})

test('pickOtherCamera picks the first other video input', () => {
  const devices = [
    { kind: 'audioinput', deviceId: 'mic' },
    { kind: 'videoinput', deviceId: 'front' },
    { kind: 'videoinput', deviceId: 'back' },
    { kind: 'videoinput', deviceId: 'wide' },
  ] as const
  assert.equal(pickOtherCamera([...devices], 'front'), 'back')
  assert.equal(pickOtherCamera([...devices]), 'front')
})

test('pickOtherCamera finds nothing with a single camera', () => {
  const devices = [{ kind: 'videoinput', deviceId: 'front' }] as const
  assert.equal(pickOtherCamera([...devices], 'front'), undefined)
})
