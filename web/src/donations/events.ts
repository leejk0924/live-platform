import type { Donation } from '../api/types.ts'
import { MAX_AMOUNT, MIN_AMOUNT } from './amount.ts'
import type { Totals } from './donationsState.ts'

const DECIMAL = /^\d+$/

// The `data` of a `donation` event, or null when it is not what the server sends.
export function parseDonation(data: string): ({ donation: Donation } & Totals) | null {
  const parsed = parseJson(data)
  if (!isTotals(parsed)) return null
  const { donation } = parsed as Record<string, unknown>
  return isDonation(donation) ? { donation, total: parsed.total, count: parsed.count } : null
}

// The `data` of a `donations` event, or null when it is not what the server sends.
export function parseTotals(data: string): Totals | null {
  const parsed = parseJson(data)
  return isTotals(parsed) ? { total: parsed.total, count: parsed.count } : null
}

// `undefined` for anything that is not JSON, which no JSON text parses to.
function parseJson(data: string): unknown {
  try {
    return JSON.parse(data) as unknown
  } catch {
    return undefined
  }
}

function isTotals(value: unknown): value is Totals {
  if (typeof value !== 'object' || value === null) return false
  const { total, count } = value as Record<string, unknown>
  return isCount(total) && isCount(count)
}

function isDonation(value: unknown): value is Donation {
  if (typeof value !== 'object' || value === null) return false
  const { id, broadcastId, donorName, amount, approvedAt } = value as Record<string, unknown>
  return (
    isDecimal(id) &&
    isDecimal(broadcastId) &&
    typeof donorName === 'string' &&
    isCount(amount) &&
    amount >= MIN_AMOUNT &&
    amount <= MAX_AMOUNT &&
    typeof approvedAt === 'string'
  )
}

function isCount(value: unknown): value is number {
  return typeof value === 'number' && Number.isSafeInteger(value) && value >= 0
}

function isDecimal(value: unknown): value is string {
  return typeof value === 'string' && DECIMAL.test(value)
}
