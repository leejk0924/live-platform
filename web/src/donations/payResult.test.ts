import assert from 'node:assert/strict'
import { test } from 'node:test'
import { failNotice, paymentStartError, readPayment } from './payResult.ts'

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

test('the fail page shows our own words, never the message in the URL', () => {
  for (const code of ['PAY_PROCESS_CANCELED', 'USER_CANCEL']) {
    const search = new URLSearchParams({ code, message: '지금 www.evil.example 로 입금하세요' })
    assert.deepEqual(failNotice(search), { text: '결제를 취소했습니다.', code })
  }
  const rejected = new URLSearchParams({ code: 'REJECT_CARD_COMPANY', message: '고객센터 010-0000-0000' })
  assert.deepEqual(failNotice(rejected), { text: '결제를 완료하지 못했습니다.', code: 'REJECT_CARD_COMPANY' })
})

test('only a code shaped like Toss codes is shown', () => {
  for (const code of ['', 'call 010-0000-0000', 'a<b>', 'X'.repeat(65), 'lower_case']) {
    assert.deepEqual(failNotice(new URLSearchParams({ code })), { text: '결제를 완료하지 못했습니다.', code: null }, code)
  }
  assert.deepEqual(failNotice(new URLSearchParams()), { text: '결제를 완료하지 못했습니다.', code: null })
})

test('a payment window that would not start says why', () => {
  assert.equal(paymentStartError({ code: 'USER_CANCEL', message: 'x' }), '결제를 취소했습니다.')
  assert.equal(paymentStartError({ code: 'NEED_CARD_PAYMENT_DETAIL', message: 'x' }), '카드사를 선택하세요.')
  assert.equal(
    paymentStartError({ code: 'NEED_AGREEMENT_WITH_REQUIRED_TERMS', message: 'x' }),
    '필수 약관에 동의하세요.',
  )
  assert.equal(
    paymentStartError({ code: 'SOMETHING_NEW', message: '결제수단을 선택해 주세요.' }),
    '결제를 시작하지 못했습니다. 결제수단을 선택해 주세요.',
  )
  for (const error of [new Error(''), null, 'boom', { code: 'SOMETHING_NEW' }]) {
    assert.equal(paymentStartError(error), '결제를 시작하지 못했습니다. 다시 시도하세요.', String(error))
  }
})
