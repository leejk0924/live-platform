import { useCallback, useEffect, useRef, useState } from 'react'
import { useNavigate, useSearchParams } from 'react-router'
import { ApiError, errorMessage } from '../api/client.ts'
import { donationApi } from '../api/donations.ts'
import type { DonationConfirmed } from '../api/types.ts'
import { formatWon } from '../donations/amount.ts'
import PayPage from '../donations/PayPage.tsx'

type Payment = { paymentKey: string; orderId: string; amount: number }

type Confirmation =
  | { status: 'confirming' }
  | { status: 'done'; result: DonationConfirmed }
  | { status: 'failed'; message: string; retryable: boolean }

export default function PaySuccess() {
  const [search] = useSearchParams()
  // Read once: the payment leaves the URL as soon as confirming starts.
  const [payment] = useState(() => readPayment(search))
  return (
    <PayPage title="후원 결제">
      {(broadcastId) =>
        payment === null ? (
          <p className="error" role="alert">
            결제 정보가 없습니다. 방송 화면에서 후원 내역을 확인하세요.
          </p>
        ) : (
          <Confirm broadcastId={broadcastId} payment={payment} />
        )
      }
    </PayPage>
  )
}

function readPayment(search: URLSearchParams): Payment | null {
  const paymentKey = search.get('paymentKey')
  const orderId = search.get('orderId')
  const amount = Number(search.get('amount'))
  if (!paymentKey || !orderId || !Number.isSafeInteger(amount)) return null
  return { paymentKey, orderId, amount }
}

function Confirm({ broadcastId, payment }: { broadcastId: string; payment: Payment }) {
  const navigate = useNavigate()
  const [confirmation, setConfirmation] = useState<Confirmation>({ status: 'confirming' })
  const started = useRef(false)

  const confirm = useCallback(async () => {
    setConfirmation({ status: 'confirming' })
    try {
      const result = await donationApi.confirm(payment.paymentKey, payment.orderId, payment.amount)
      setConfirmation({ status: 'done', result })
    } catch (e) {
      const retryable = !(e instanceof ApiError) || e.code === 'PAYMENT_GATEWAY_UNAVAILABLE' || e.status >= 500
      setConfirmation({ status: 'failed', message: errorMessage(e), retryable })
    }
  }, [payment])

  // Once, though StrictMode runs effects twice: the server would answer a second confirm with
  // `credited: false`, but there is no reason to ask. The payment key leaves the address bar and
  // history, and with it the Referer of the requests that follow.
  useEffect(() => {
    if (started.current) return
    started.current = true
    navigate(`/pay/success?${new URLSearchParams({ broadcast: broadcastId })}`, { replace: true })
    void confirm()
  }, [broadcastId, confirm, navigate])

  switch (confirmation.status) {
    case 'confirming':
      return <p>결제를 확인하는 중…</p>
    case 'done': {
      const { donation } = confirmation.result
      return (
        <p className="notice" role="status">
          {formatWon(donation.amount)}을 후원했습니다. 고맙습니다!
        </p>
      )
    }
    case 'failed':
      return (
        <p className="error" role="alert">
          결제를 마치지 못했습니다. {confirmation.message}{' '}
          {confirmation.retryable && (
            <button type="button" onClick={() => void confirm()}>
              다시 시도
            </button>
          )}
        </p>
      )
  }
}
