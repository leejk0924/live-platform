import assert from 'node:assert/strict'
import { test } from 'node:test'
import type { Donation } from '../api/types.ts'
import {
  donationsReducer,
  initialDonations,
  MAX_SHOWN,
  needsReload,
  type DonationsState,
} from './donationsState.ts'

const donation = (id: string, amount = 1000): Donation => ({
  id,
  broadcastId: '1',
  donorName: `viewer${id}`,
  amount,
  approvedAt: '2026-11-09T00:00:00Z',
})

const loaded = (items: Donation[], total: number, count: number, state = initialDonations): DonationsState =>
  donationsReducer(state, { type: 'loaded', page: { items, total, count, nextBefore: null } })
const donated = (state: DonationsState, d: Donation, total: number, count: number) =>
  donationsReducer(state, { type: 'donated', donation: d, total, count })
const summary = (state: DonationsState, total: number, count: number) =>
  donationsReducer(state, { type: 'totals', total, count })
const ids = (state: DonationsState) => state.items.map((d) => d.id)

test('the loaded page shows newest first with its totals', () => {
  const state = loaded([donation('3'), donation('2')], 2000, 2)
  assert.deepEqual(ids(state), ['3', '2'])
  assert.equal(state.total, 2000)
  assert.equal(state.count, 2)
  assert.equal(needsReload(state), false)
})

test('a live donation goes on top, raises the totals and is announced', () => {
  const state = donated(loaded([donation('2')], 1000, 1), donation('5', 5000), 6000, 2)
  assert.deepEqual(ids(state), ['5', '2'])
  assert.equal(state.total, 6000)
  assert.equal(state.count, 2)
  assert.equal(state.fresh?.id, '5')
  assert.equal(needsReload(state), false)
})

test('the same donation twice is listed and announced once', () => {
  const once = donated(loaded([], 0, 0), donation('5'), 1000, 1)
  const twice = donated(donationsReducer(once, { type: 'faded', id: '5' }), donation('5'), 1000, 1)
  assert.deepEqual(ids(twice), ['5'])
  assert.equal(twice.fresh, null)
  assert.equal(needsReload(twice), false)
})

test('ids order as numbers, not strings', () => {
  const state = donated(loaded([donation('9')], 1000, 1), donation('10'), 2000, 2)
  assert.deepEqual(ids(state), ['10', '9'])
})

test('totals never go down, whatever order they arrive in', () => {
  const state = donated(loaded([], 0, 0), donation('5', 5000), 5000, 1)
  const stale = summary(state, 0, 0)
  assert.equal(stale.total, 5000)
  assert.equal(stale.count, 1)
  const reloadedEarlier = loaded([], 0, 0, stale)
  assert.equal(reloadedEarlier.total, 5000)
})

test('a summary counting more than the list holds asks for a reload', () => {
  const state = summary(loaded([donation('2')], 1000, 1), 3000, 3)
  assert.equal(state.total, 3000)
  assert.equal(needsReload(state), true)
  assert.equal(needsReload(loaded([donation('4'), donation('3'), donation('2')], 3000, 3, state)), false)
})

test('a donation whose count skips one means one was missed', () => {
  const state = donated(loaded([donation('2')], 1000, 1), donation('7'), 3000, 3)
  assert.equal(needsReload(state), true)
})

test('a donation that arrived while the page was read survives the reload', () => {
  const live = donated(loaded([], 0, 0), donation('9'), 1000, 1)
  const reloaded = loaded([donation('3')], 1000, 1, live)
  assert.deepEqual(ids(reloaded), ['9', '3'])
  assert.equal(reloaded.count, 1)
  assert.equal(reloaded.listed, 2)
})

test('the list keeps only the newest few', () => {
  let state = loaded([], 0, 0)
  for (let i = 1; i <= MAX_SHOWN + 5; i++) state = donated(state, donation(String(i)), i * 1000, i)
  assert.equal(state.items.length, MAX_SHOWN)
  assert.equal(state.items[0]?.id, String(MAX_SHOWN + 5))
  assert.equal(needsReload(state), false)
})

test('the notice fades only for the donation it shows', () => {
  const first = donated(loaded([], 0, 0), donation('1'), 1000, 1)
  const second = donated(first, donation('2'), 2000, 2)
  assert.equal(donationsReducer(second, { type: 'faded', id: '1' }).fresh?.id, '2')
  assert.equal(donationsReducer(second, { type: 'faded', id: '2' }).fresh, null)
})
