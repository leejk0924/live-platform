// Feature: donation (features/donation.md). Needs internet access to the Toss sandbox and test keys in .env.
// The owner goes live. An anonymous watcher sees 0원, and the owner sees no donation form. A signed-in donor picks
// 5,000원, pays in the Toss sandbox widget (quick account transfer, PIN 000000 on Toss's keypad), and lands on
// /pay/success. The watcher sees the donation arrive live. The api logs payment_confirmed without the paymentKey.
import { execFileSync } from 'node:child_process'
import {
  BASE, REPO, RUN_ID, log, check, must, shot, launchBrowser, newContext, account, createBroadcast,
  startPublishing, stopBroadcast, noCspViolations, summary,
} from '../lib.mjs'

const browser = await launchBrowser()
const startedAt = new Date().toISOString()
let ownerPage, id
let paymentKey // kept in memory only, to prove the logs never contain it
const totals = (page) => page.locator('.donation-totals').innerText()
const cspHeader = (page) => page.evaluate(() => fetch(location.href).then((r) => r.headers.get('content-security-policy') ?? ''))
const tossFrames = (page) => page.frames().filter((f) => f.url().includes('toss.im'))

// Toss's sandbox window lives in cross-origin toss.im frames. Quick account transfer asks for 다음/동의, a 6-digit PIN
// twice on a keypad that needs real mouse events, then 결제하기. Proven with the #13 run; Toss may change its UI.
async function payInTossWindow(page) {
  let typed = ''
  for (let step = 0; step < 30; step++) {
    const pathname = new URL(page.url()).pathname
    if (pathname.startsWith('/pay/') && pathname !== '/pay/checkout') return pathname
    const frames = tossFrames(page)
    // A click that lands before the widget is ready opens nothing: click 결제하기 again.
    if (step % 4 === 3 && frames.length === 0 && (await page.getByRole('button', { name: '결제하기', exact: true }).isEnabled().catch(() => false))) {
      log('toss step', step, 'no payment window yet; clicking 결제하기 again')
      await page.getByRole('button', { name: '결제하기', exact: true }).click().catch(() => {})
    }
    for (const frame of frames) {
      const text = await frame.evaluate(() => document.body?.innerText ?? '').catch(() => '')
      if (text.includes('000000') && typed !== text.slice(0, 12)) {
        let pressed = 0
        for (let tries = 0; pressed < 6 && tries < 20; tries++) {
          const key = await frame.evaluateHandle(() => {
            const keys = [...document.querySelectorAll('*')]
              .filter((e) => (e.innerText || '').trim() === '0' && e.getBoundingClientRect().width > 0)
              .sort((a, b) => a.getBoundingClientRect().width * a.getBoundingClientRect().height - b.getBoundingClientRect().width * b.getBoundingClientRect().height)
            return keys[0] ?? null
          }).catch(() => null)
          const el = key?.asElement()
          if (!el) { await page.waitForTimeout(500); continue }
          await el.click({ timeout: 3000 }).then(() => pressed++).catch(() => {})
          await page.waitForTimeout(300)
        }
        if (pressed === 6) typed = text.slice(0, 12)
        continue
      }
      const next = await frame.evaluate(() => {
        const b = [...document.querySelectorAll('button')].filter((b) => b.offsetParent !== null && /다음|결제하기|확인|동의|완료|이체/.test(b.textContent.trim())).at(-1)
        if (!b) return null
        b.click()
        return b.textContent.trim()
      }).catch(() => null)
      if (next) log('toss step', step, 'click', next)
    }
    await page.waitForTimeout(3000)
  }
  return new URL(page.url()).pathname
}

try {
  const owner = await newContext(browser, 'owner')
  ownerPage = await owner.newPage()
  const o = await account(ownerPage, 'streamer', { streamer: true })
  id = await createBroadcast(ownerPage, `verify donation ${RUN_ID}`)
  log('owner', o.username, 'broadcast', id)
  must(await startPublishing(ownerPage, id), 'owner publishes and the broadcast turns LIVE')
  const broadcastUrl = `${BASE}/broadcasts/${id}`
  check((await ownerPage.locator('.donation-form').count()) === 0, 'owner gets no donation form on its own broadcast')

  const watcher = await newContext(browser, 'watcher')
  const watcherPage = await watcher.newPage()
  await watcherPage.goto(broadcastUrl)
  await watcherPage.locator('.donation-totals').getByText(/^0원/).waitFor({ timeout: 15_000 })
  check(true, `anonymous watcher sees totals "${await totals(watcherPage)}"`)

  const donor = await newContext(browser, 'donor')
  const donorPage = await donor.newPage()
  donorPage.on('framenavigated', (f) => {
    if (f === donorPage.mainFrame() && f.url().includes('paymentKey=')) paymentKey = new URL(f.url()).searchParams.get('paymentKey')
  })
  const d = await account(donorPage, 'donor')
  await donorPage.goto(broadcastUrl)
  const form = donorPage.locator('.donation-form')
  await form.getByRole('button', { name: '5,000원' }).click()
  await form.getByRole('button', { name: '후원하기' }).click()
  await donorPage.waitForURL('**/pay/checkout**', { timeout: 15_000 })
  const payCsp = await cspHeader(donorPage)
  check(payCsp.includes('https://js.tosspayments.com') && payCsp.includes("frame-ancestors 'none'"), '/pay/checkout carries the Toss CSP')
  const pay = donorPage.getByRole('button', { name: '결제하기', exact: true })
  await pay.waitFor({ timeout: 45_000 })
  await shot(donorPage, '1-checkout-widget')
  await pay.click()

  const landed = await payInTossWindow(donorPage)
  must(landed === '/pay/success', `Toss sandbox returns to ${landed}`)
  await donorPage.getByText('5,000원을 후원했습니다').waitFor({ timeout: 30_000 })
  check(true, 'success page confirms 5,000원')
  check(!donorPage.url().includes('paymentKey'), 'paymentKey left the address bar')
  await shot(donorPage, '2-pay-success')

  await watcherPage.locator('.donation-fresh').getByText('5,000원').waitFor({ timeout: 15_000 })
  check(true, `watcher sees the donation live: "${await watcherPage.locator('.donation-fresh').innerText()}"`)
  check((await totals(watcherPage)).startsWith('5,000원'), `watcher totals: "${await totals(watcherPage)}"`)
  await shot(watcherPage, '3-watcher-live-donation')

  await donorPage.goto(broadcastUrl)
  await donorPage.locator('.donation-totals').getByText(/^5,000원/).waitFor({ timeout: 15_000 })
  const listed = await donorPage.locator('.donation-list .donation').allInnerTexts()
  check(listed.length === 1 && listed[0].includes(d.username) && listed[0].includes('5,000원'), `donation listed: ${JSON.stringify(listed)}`)
  check(!(await cspHeader(donorPage)).includes('tosspayments'), 'broadcast page is back on the strict CSP')

  // Audit: payment_confirmed is logged, and the paymentKey appears nowhere in the api log.
  const apiLog = execFileSync('docker', ['compose', 'logs', 'api', '--since', startedAt], { cwd: REPO, maxBuffer: 64 << 20 }).toString()
  check(/payment_confirmed/.test(apiLog), 'api logs payment_confirmed')
  must(paymentKey, 'the success redirect carried a paymentKey')
  check(!apiLog.includes(paymentKey) && !/paymentKey=/.test(apiLog), 'api log never contains the paymentKey')

  await noCspViolations([['owner', ownerPage], ['watcher', watcherPage], ['donor', donorPage]])
} catch (e) {
  check(false, `journey aborted: ${e.message.split('\n')[0]}`)
} finally {
  if (ownerPage && id) await stopBroadcast(ownerPage, id)
  await browser.close()
}
process.exit(summary() ? 0 : 1)
