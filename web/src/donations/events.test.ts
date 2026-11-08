import assert from 'node:assert/strict'
import { test } from 'node:test'
import { parseAmount } from './amount.ts'
import { parseDonation, parseTotals } from './events.ts'

const donation = { id: '5', broadcastId: '1', donorName: '<b>viewer</b>', amount: 5000, approvedAt: '2026-11-09T00:00:00Z' }

test('a donation event is a donation with the totals that include it', () => {
  const data = JSON.stringify({ donation, total: 6000, count: 2 })
  assert.deepEqual(parseDonation(data), { donation, total: 6000, count: 2 })
})

test('a malformed donation event is refused', () => {
  const malformed = [
    { ...donation, id: 5 },
    { ...donation, id: '' },
    { ...donation, broadcastId: '-1' },
    { ...donation, donorName: null },
    { ...donation, amount: 999 },
    { ...donation, amount: 1_000_001 },
    { ...donation, amount: 1000.5 },
    { ...donation, amount: '5000' },
    { ...donation, approvedAt: undefined },
  ]
  const events = [
    ...malformed.map((d) => ({ donation: d, total: 6000, count: 2 })),
    { donation, total: -1, count: 2 },
    { donation, total: 6000, count: 1.5 },
    { donation, total: 6000 },
    { total: 6000, count: 2 },
  ]
  for (const data of ['', 'not json', 'null', '[]', ...events.map((e) => JSON.stringify(e))]) {
    assert.equal(parseDonation(data), null, data)
  }
})

test('a donations event is a whole, non-negative total and count', () => {
  assert.deepEqual(parseTotals('{"total": 0, "count": 0}'), { total: 0, count: 0 })
  assert.deepEqual(parseTotals('{"total": 15000, "count": 3, "extra": 1}'), { total: 15000, count: 3 })
  for (const data of ['', 'null', '{}', '{"total": 1}', '{"total": -1, "count": 0}', '{"total": "1", "count": 0}', '{"total": 1, "count": 0.5}']) {
    assert.equal(parseTotals(data), null, data)
  }
})

test('an amount is whole won from 1,000 to 1,000,000', () => {
  for (const [text, amount] of [['1000', 1000], ['5000', 5000], ['1000000', 1_000_000]] as const) {
    assert.equal(parseAmount(text), amount)
  }
  for (const text of [null, '', '999', '1000001', '1000.0', '1e4', ' 5000', '-5000', '5,000']) {
    assert.equal(parseAmount(text), null, String(text))
  }
})
