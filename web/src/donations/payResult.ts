// What Toss hands the /pay pages in their URLs, read so that nothing in the URL is trusted.
import { parseAmount } from './amount.ts'

export type Payment = { paymentKey: string; orderId: string; amount: number }

// The payment Toss authorized, or null when the URL lacks a part or holds an amount no order has.
export function readPayment(search: URLSearchParams): Payment | null {
  const paymentKey = search.get('paymentKey')
  const orderId = search.get('orderId')
  const amount = parseAmount(search.get('amount'))
  if (!paymentKey || !orderId || amount === null) return null
  return { paymentKey, orderId, amount }
}

const CANCEL_CODES = new Set(['PAY_PROCESS_CANCELED', 'USER_CANCEL'])
const TOSS_CODE = /^[A-Z0-9_]{1,64}$/

// Anyone can link to the fail page with any `message`, so only our own text is shown, and the code
// only when it looks like one of Toss's.
export function failNotice(search: URLSearchParams): { text: string; code: string | null } {
  const raw = search.get('code')
  const code = raw !== null && TOSS_CODE.test(raw) ? raw : null
  const text = code !== null && CANCEL_CODES.has(code) ? '결제를 취소했습니다.' : '결제를 완료하지 못했습니다.'
  return { text, code }
}
