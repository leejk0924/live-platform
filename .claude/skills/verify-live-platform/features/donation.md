# Donate with Toss (test mode)

A signed-in viewer of a LIVE broadcast picks an amount, pays through the Toss payment widget on `/pay/checkout`, and lands on `/pay/success`. The server confirms the payment with Toss and credits it exactly once. Everyone on the broadcast page sees the donation and the new total in real time.

## Sub-features
- Order creation: 1,000 to 1,000,000 won, only for LIVE or INTERRUPTED broadcasts, never the owner's own. One order per user per 3 s (429).
- Confirm:
  - An amount mismatch is rejected before Toss is called.
  - Toss confirm uses `Idempotency-Key: donation-confirm-<orderId>`.
  - The order then moves `PENDING → DONE` with a conditional UPDATE.
  - The same confirm sent 10 times credits once.
- Webhook: Toss is re-queried by `paymentKey`, never trusting the body. Webhooks are limited to 60 per minute per client.
- Live display: `.donation-totals` (for example `5,000원 · 1건`), `.donation-fresh` notice, `.donation-list`. These go over the comment SSE stream's priority lane.
- Audit log: `donation_ordered`, `payment_confirmed`, `payment_rejected`. The log never contains a paymentKey, a secret or an Authorization header.
- `/pay` alone gets the Toss CSP. Every other page keeps the strict CSP.

## How to get to it (user POV)
1. Log in as someone other than the owner.
2. Open a LIVE broadcast. In `section.donations[aria-label="후원"]`, pick `5,000원` and press `후원하기`.
3. Pay in the Toss widget on `/pay/checkout`. The app redirects to `/pay/success`, which shows `후원했습니다`.

## Driving it with Playwright
`bash scripts/run.sh donation` ([journeys/donation.mjs](../scripts/journeys/donation.mjs)). Needs internet access to the Toss sandbox.
1. The owner (role `streamer`) goes live. The owner's page has no `.donation-form`.
2. An anonymous watcher opens the page, and `.donation-totals` shows `0원 · 0건`.
3. The donor (role `donor`) clicks `5,000원` and then `후원하기`.
   - Expect `/pay/checkout`, with a CSP header containing `https://js.tosspayments.com` and `frame-ancestors 'none'`.
   - Expect the `결제하기` button within 45 s.
4. Click `결제하기`. The Toss window lives in cross-origin `toss.im` frames (`page.frames()`).
   - The journey clicks `결제하기` again if no frame opened.
   - It clicks the visible `다음`/`동의`/`결제하기` buttons in the frames.
   - If a keypad shows `000000`, it presses the smallest `0` key six times with real mouse clicks.
   - The 2026-11-10 run needed no PIN: `다음` and then `결제하기`.
5. Expect `/pay/success` with `5,000원을 후원했습니다`, and no `paymentKey` left in the URL.
6. The watcher shows `.donation-fresh` with `5,000원`, and totals of `5,000원 · 1건`, within 15 s. The donor's broadcast page lists the donation and is back on the strict CSP.
7. Side effect: `docker compose logs api --since <start>` has `payment_confirmed`.
   - The paymentKey captured from the redirect (kept in memory, never logged) appears nowhere in that log.
   - `paymentKey=` is redacted in every evidence log line.

End state that proves it: the success page, the live total in another context, the audit line without the key, and no CSP violations in any context, `/pay/*` included.

## Gotchas
- This journey needs internet access to the Toss sandbox and `TOSS_CLIENT_KEY`/`TOSS_SECRET_KEY` (`test_gck_`/`test_gsk_`) in `.env`. The server refuses non-`test_` keys. No real money moves.
- The current `.env` keys are Toss's public docs keys (`test_*_docs_*`). Payments do not show up in a personal dashboard, and `GET /v1/payments/orders/{orderId}` answers `NOT_FOUND_MERCHANT`. The app looks payments up only by paymentKey.
- Proven only with the sandbox's default flow (quick account transfer in the #13 run). Completing a card payment, and the `/pay/fail` page, are unverified. The claim that a card decline comes back as 403 rests on the docs, not on a real decline.
- Never log or screenshot the redirect URL with `paymentKey=` in evidence. Redact it the way the #13 run did.
