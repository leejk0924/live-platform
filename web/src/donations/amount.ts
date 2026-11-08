// The server's bounds, in won.
export const MIN_AMOUNT = 1000
export const MAX_AMOUNT = 1_000_000

export const PRESET_AMOUNTS = [1000, 5000, 10_000, 50_000] as const

const DIGITS = /^\d+$/

// A donation amount typed or passed in a URL, or null when the server would refuse it.
export function parseAmount(text: string | null): number | null {
  if (text === null || !DIGITS.test(text)) return null
  const amount = Number(text)
  return amount >= MIN_AMOUNT && amount <= MAX_AMOUNT ? amount : null
}

export const formatWon = (amount: number): string => `${amount.toLocaleString('ko-KR')}원`
