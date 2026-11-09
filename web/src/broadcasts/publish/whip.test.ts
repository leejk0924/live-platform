import assert from 'node:assert/strict'
import { test } from 'node:test'
import { scaleDownBy } from './whip.ts'

test('a portrait 720x1280 phone frame is sent at full size', () => {
  assert.equal(scaleDownBy({ width: 720, height: 1280 }), 1)
})

test('a landscape 1080p screen is scaled down to 720p', () => {
  assert.equal(scaleDownBy({ width: 1920, height: 1080 }), 1.5)
})

test('a frame smaller than 720p is never scaled up', () => {
  assert.equal(scaleDownBy({ width: 640, height: 480 }), 1)
})

test('unknown dimensions send the frame as it is', () => {
  assert.equal(scaleDownBy({}), 1)
})
