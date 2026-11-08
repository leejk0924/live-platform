import assert from 'node:assert/strict'
import { test } from 'node:test'
import { checkoutUrl, failUrl, parseBroadcastId, successUrl } from './payUrls.ts'

test('checkout carries the broadcast and amount under /pay', () => {
  const url = new URL(checkoutUrl('112007302751453184', 5000), 'https://localhost')
  assert.equal(url.pathname, '/pay/checkout')
  assert.equal(parseBroadcastId(url.searchParams), '112007302751453184')
  assert.equal(url.searchParams.get('amount'), '5000')
})

test('Toss returns to /pay pages that still know the broadcast', () => {
  for (const make of [successUrl, failUrl]) {
    const url = new URL(make('https://localhost', '42'))
    assert.equal(url.origin, 'https://localhost')
    assert.match(url.pathname, /^\/pay\//)
    assert.equal(parseBroadcastId(url.searchParams), '42')
  }
})

test('a broadcast id from the URL is decimal or nothing', () => {
  for (const search of ['', 'broadcast=', 'broadcast=abc', 'broadcast=1%2F2', 'broadcast=-1']) {
    assert.equal(parseBroadcastId(new URLSearchParams(search)), null, search)
  }
})
