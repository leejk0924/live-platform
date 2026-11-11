import assert from 'node:assert/strict'
import { test } from 'node:test'
import { commentTime } from './commentTime.ts'

const now = new Date(2026, 10, 12, 21, 30)
const at = (...parts: [number, number, number, number, number]) => new Date(...parts).toISOString()

test('a comment from today shows only the time', () => {
  assert.equal(commentTime(at(2026, 10, 12, 9, 5), now).text, '09:05')
})

test('a comment from another day also shows the day', () => {
  assert.equal(commentTime(at(2026, 10, 9, 23, 59), now).text, '11월 9일 23:59')
})

test('the same time of day a year earlier is not today', () => {
  assert.equal(commentTime(at(2025, 10, 12, 21, 0), now).text, '11월 12일 21:00')
})

test('the title carries the year the text leaves out', () => {
  assert.match(commentTime(at(2025, 10, 12, 21, 0), now).title, /2025/)
})
