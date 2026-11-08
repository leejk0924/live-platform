// The part of Toss Payments' widget SDK v2 (https://docs.tosspayments.com/sdk/v2/js) this app uses.
// The SDK is loaded from Toss's CDN, as Toss requires, and only on /pay pages, whose CSP allows it.
export type TossWidgets = {
  setAmount(amount: { currency: 'KRW'; value: number }): Promise<void>
  renderPaymentMethods(options: { selector: string; variantKey: string }): Promise<unknown>
  renderAgreement(options: { selector: string; variantKey: string }): Promise<unknown>
  requestPayment(options: { orderId: string; orderName: string; successUrl: string; failUrl: string }): Promise<void>
}

type TossPayments = ((clientKey: string) => { widgets(options: { customerKey: string }): TossWidgets }) & {
  ANONYMOUS: string
}

declare global {
  interface Window {
    TossPayments?: TossPayments
  }
}

const SDK_URL = 'https://js.tosspayments.com/v2/standard'

let loading: Promise<TossPayments> | undefined

export function loadTossPayments(): Promise<TossPayments> {
  loading ??= new Promise<TossPayments>((resolve, reject) => {
    const script = document.createElement('script')
    script.src = SDK_URL
    script.onload = () => (window.TossPayments ? resolve(window.TossPayments) : reject(new Error('no TossPayments')))
    script.onerror = () => reject(new Error('SDK load failed'))
    document.head.append(script)
  }).catch((error: unknown) => {
    // A failed load is not cached, so trying again inserts the script again.
    loading = undefined
    throw error
  })
  return loading
}

// Toss rejects with `{ code, message }`; `USER_CANCEL` when the buyer closes the window.
export function tossErrorCode(error: unknown): string | undefined {
  if (typeof error !== 'object' || error === null) return undefined
  const { code } = error as Record<string, unknown>
  return typeof code === 'string' ? code : undefined
}
