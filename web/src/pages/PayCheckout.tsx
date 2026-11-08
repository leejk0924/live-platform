import { useEffect, useRef, useState } from 'react'
import { ApiError, errorMessage } from '../api/client.ts'
import { donationApi } from '../api/donations.ts'
import type { DonationOrder } from '../api/types.ts'
import { formatWon, parseAmount } from '../donations/amount.ts'
import PayPage from '../donations/PayPage.tsx'
import { failUrl, successUrl } from '../donations/payUrls.ts'
import { loadTossPayments, tossErrorCode, type TossWidgets } from '../donations/toss.ts'

const METHODS_ID = 'payment-methods'
const AGREEMENT_ID = 'payment-agreement'

type Checkout =
  | { status: 'loading' }
  | { status: 'ready'; order: DonationOrder; widgets: TossWidgets }
  | { status: 'failed'; message: string }

export default function PayCheckout() {
  return (
    <PayPage title="후원하기">
      {(broadcastId, search) => {
        const amount = parseAmount(search.get('amount'))
        if (amount === null) {
          return (
            <p className="error" role="alert">
              후원 금액은 1,000원에서 1,000,000원 사이여야 합니다.
            </p>
          )
        }
        return <Checkout broadcastId={broadcastId} amount={amount} />
      }}
    </PayPage>
  )
}

function Checkout({ broadcastId, amount }: { broadcastId: string; amount: number }) {
  const [checkout, setCheckout] = useState<Checkout>({ status: 'loading' })
  const [paying, setPaying] = useState(false)
  const [payError, setPayError] = useState<string | null>(null)
  // StrictMode runs effects twice; a second run would order twice and render the widget twice
  // into the same element.
  const started = useRef(false)

  useEffect(() => {
    if (started.current) return
    started.current = true
    void (async () => {
      try {
        const order = await donationApi.order(broadcastId, amount)
        const TossPayments = await loadTossPayments()
        const widgets = TossPayments(order.clientKey).widgets({ customerKey: TossPayments.ANONYMOUS })
        await widgets.setAmount({ currency: 'KRW', value: order.amount })
        await Promise.all([
          widgets.renderPaymentMethods({ selector: `#${METHODS_ID}`, variantKey: 'DEFAULT' }),
          widgets.renderAgreement({ selector: `#${AGREEMENT_ID}`, variantKey: 'AGREEMENT' }),
        ])
        setCheckout({ status: 'ready', order, widgets })
      } catch (e) {
        const message = e instanceof ApiError ? errorMessage(e) : '결제창을 불러오지 못했습니다. 잠시 후 다시 시도하세요.'
        setCheckout({ status: 'failed', message })
      }
    })()
  }, [broadcastId, amount])

  async function pay() {
    if (checkout.status !== 'ready') return
    setPaying(true)
    setPayError(null)
    const { order, widgets } = checkout
    try {
      // On success the browser leaves for the success URL.
      await widgets.requestPayment({
        orderId: order.orderId,
        orderName: order.orderName,
        successUrl: successUrl(window.location.origin, broadcastId),
        failUrl: failUrl(window.location.origin, broadcastId),
      })
    } catch (e) {
      setPayError(tossErrorCode(e) === 'USER_CANCEL' ? '결제를 취소했습니다.' : '결제를 시작하지 못했습니다. 다시 시도하세요.')
    } finally {
      setPaying(false)
    }
  }

  return (
    <>
      <p>
        후원 금액 <strong>{formatWon(amount)}</strong>
      </p>
      {checkout.status === 'loading' && <p>결제창을 불러오는 중…</p>}
      {checkout.status === 'failed' && (
        <p className="error" role="alert">
          {checkout.message}
        </p>
      )}
      <div id={METHODS_ID} />
      <div id={AGREEMENT_ID} />
      {checkout.status === 'ready' && (
        <button type="button" onClick={() => void pay()} disabled={paying}>
          결제하기
        </button>
      )}
      {payError && (
        <p className="error" role="alert">
          {payError}
        </p>
      )}
    </>
  )
}
