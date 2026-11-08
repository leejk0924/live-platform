import { useState, type FormEvent } from 'react'
import { errorMessage } from '../api/client.ts'
import type { Broadcast } from '../api/types.ts'
import { useAuth } from '../auth/context.ts'
import { formatWon, MAX_AMOUNT, MIN_AMOUNT, parseAmount, PRESET_AMOUNTS } from './amount.ts'
import type { DonationsState } from './donationsState.ts'
import { checkoutUrl } from './payUrls.ts'

export default function DonationPanel({
  broadcast,
  state,
  loadError,
  onRetry,
}: {
  broadcast: Broadcast
  state: DonationsState
  loadError: unknown
  onRetry: () => void
}) {
  const { auth } = useAuth()
  const onAir = broadcast.status === 'LIVE' || broadcast.status === 'INTERRUPTED'
  const canDonate = onAir && auth.status === 'signedIn' && auth.user.id !== broadcast.ownerId

  return (
    <section className="donations" aria-label="후원">
      <div className="donations-header">
        <h2>후원</h2>
        <span className="donation-totals">
          {formatWon(state.total)} · {state.count.toLocaleString('ko-KR')}건
        </span>
      </div>
      {state.fresh && (
        <p className="donation-fresh" role="status">
          {state.fresh.donorName}님이 {formatWon(state.fresh.amount)}을 후원했습니다!
        </p>
      )}
      {loadError !== null && (
        <p className="error" role="alert">
          후원 내역을 불러오지 못했습니다. {errorMessage(loadError)}{' '}
          <button type="button" onClick={onRetry}>
            다시 시도
          </button>
        </p>
      )}
      {state.items.length > 0 && (
        <ol className="donation-list">
          {state.items.map((donation) => (
            <li key={donation.id} className="donation">
              <span className="donation-donor">{donation.donorName}</span>
              <span className="donation-amount">{formatWon(donation.amount)}</span>
            </li>
          ))}
        </ol>
      )}
      {canDonate && <DonationForm broadcastId={broadcast.id} />}
    </section>
  )
}

function DonationForm({ broadcastId }: { broadcastId: string }) {
  const [text, setText] = useState(String(PRESET_AMOUNTS[1]))
  const [error, setError] = useState<string | null>(null)

  function onSubmit(e: FormEvent) {
    e.preventDefault()
    const amount = parseAmount(text)
    if (amount === null) {
      setError(`후원 금액은 ${formatWon(MIN_AMOUNT)}에서 ${formatWon(MAX_AMOUNT)} 사이의 원 단위로 입력하세요.`)
      return
    }
    // A full page load, not router navigation: the checkout page's CSP allows Toss.
    window.location.assign(checkoutUrl(broadcastId, amount))
  }

  return (
    <form className="donation-form" onSubmit={onSubmit}>
      <div className="donation-presets">
        {PRESET_AMOUNTS.map((amount) => (
          <button
            key={amount}
            type="button"
            aria-pressed={text === String(amount)}
            onClick={() => {
              setText(String(amount))
              setError(null)
            }}
          >
            {formatWon(amount)}
          </button>
        ))}
      </div>
      <div className="donation-input">
        <input
          aria-label="후원 금액(원)"
          type="number"
          inputMode="numeric"
          min={MIN_AMOUNT}
          max={MAX_AMOUNT}
          step={1}
          value={text}
          onChange={(e) => setText(e.target.value)}
        />
        <button type="submit">후원하기</button>
      </div>
      {error && (
        <p className="error" role="alert">
          {error}
        </p>
      )}
    </form>
  )
}
