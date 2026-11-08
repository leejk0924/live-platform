import assert from 'node:assert/strict'
import { test } from 'node:test'
import { readPayment } from './payResult.ts'

test('the success URL holds a payment only with its key, order and a valid amount', () => {
  assert.deepEqual(readPayment(new URLSearchParams('paymentKey=tgen_1&orderId=42&amount=5000')), {
    paymentKey: 'tgen_1',
    orderId: '42',
    amount: 5000,
  })
  for (const search of [
    'paymentKey=tgen_1&orderId=42',
    'paymentKey=tgen_1&orderId=42&amount=',
    'paymentKey=tgen_1&orderId=42&amount=abc',
    'paymentKey=tgen_1&orderId=42&amount=0',
    'paymentKey=tgen_1&orderId=42&amount=5000.5',
    'orderId=42&amount=5000',
    'paymentKey=tgen_1&amount=5000',
  ]) {
    assert.equal(readPayment(new URLSearchParams(search)), null, search)
  }
})
