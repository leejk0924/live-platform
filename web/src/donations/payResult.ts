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
