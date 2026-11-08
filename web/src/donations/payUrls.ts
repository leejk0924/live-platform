const DECIMAL = /^\d+$/

// Pages under /pay get the CSP that allows Toss Payments, and CSP is fixed when a document loads.
// So these are entered and left by full page loads, never by router navigation: the broadcast
// page assigns this URL, and the payment pages link back with plain anchors.
export const checkoutUrl = (broadcastId: string, amount: number): string =>
  `/pay/checkout?${new URLSearchParams({ broadcast: broadcastId, amount: String(amount) })}`

// Toss appends `paymentKey`, `orderId`, `amount` and `paymentType` to the success URL, and
// `code`, `message` and `orderId` to the fail URL.
export const successUrl = (origin: string, broadcastId: string): string =>
  `${origin}/pay/success?${new URLSearchParams({ broadcast: broadcastId })}`
export const failUrl = (origin: string, broadcastId: string): string =>
  `${origin}/pay/fail?${new URLSearchParams({ broadcast: broadcastId })}`

export const broadcastUrl = (broadcastId: string): string => `/broadcasts/${encodeURIComponent(broadcastId)}`

export function parseBroadcastId(search: URLSearchParams): string | null {
  const id = search.get('broadcast')
  return id !== null && DECIMAL.test(id) ? id : null
}
