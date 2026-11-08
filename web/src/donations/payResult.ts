// What Toss hands the /pay pages in their URLs, read so that nothing in the URL is trusted.
import { parseAmount } from './amount.ts'
import { tossErrorCode } from './toss.ts'

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

// What the buyer must do when Toss's payment window refuses to open.
const START_ERRORS: Record<string, string> = {
  USER_CANCEL: '결제를 취소했습니다.',
  NEED_CARD_PAYMENT_DETAIL: '카드사를 선택하세요.',
  NEED_AGREEMENT_WITH_REQUIRED_TERMS: '필수 약관에 동의하세요.',
}

// The SDK's own message for codes we do not know: it comes from the SDK, not the URL, and React
// renders it as text.
export function paymentStartError(error: unknown): string {
  const code = tossErrorCode(error)
  if (code !== undefined && Object.hasOwn(START_ERRORS, code)) return START_ERRORS[code]
  const message = code === undefined ? undefined : (error as { message?: unknown }).message
  return typeof message === 'string' && message.trim() !== ''
    ? `결제를 시작하지 못했습니다. ${message.trim().slice(0, 200)}`
    : '결제를 시작하지 못했습니다. 다시 시도하세요.'
}
