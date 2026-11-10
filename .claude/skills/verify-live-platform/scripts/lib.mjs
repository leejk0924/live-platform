// Shared harness for verify-live-platform journeys: Playwright (playwright-core) driving the system Chrome
// against the local compose stack at https://localhost. Journeys import this file; run them through run.sh.
import { createRequire } from 'node:module'
import { randomBytes } from 'node:crypto'
import { appendFileSync, existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const home = process.env.VERIFY_HOME
const runDir = process.env.VERIFY_RUN_DIR
if (!home || !runDir) {
  console.error('VERIFY_HOME and VERIFY_RUN_DIR are unset: start journeys with scripts/run.sh <journey>')
  process.exit(2)
}
mkdirSync(runDir, { recursive: true })

// playwright-core lives in $VERIFY_HOME (scripts/setup.sh), not in the repo.
const require = createRequire(path.join(home, 'package.json'))
const { chromium, devices } = require('playwright-core')
export { devices }

export const BASE = process.env.VERIFY_BASE ?? 'https://localhost'
export const CHROME = process.env.CHROME ?? 'C:/Program Files/Google/Chrome/Application/chrome.exe'
export const RUN_ID = path.basename(runDir)

export const REPO = fileURLToPath(new URL('../../../../', import.meta.url))

// Evidence must never hold a Toss paymentKey: every log line goes through this.
const redact = (v) => (typeof v === 'string' ? v.replace(/paymentKey=[^&"\s]*/g, 'paymentKey=<redacted>') : v)
export const log = (...a) => console.log(new Date().toISOString().slice(11, 19), ...a.map(redact))

const results = []
export function check(ok, what) {
  results.push({ ok, what })
  log(ok ? 'PASS' : 'FAIL', what)
  appendFileSync(path.join(runDir, 'checks.tsv'), `${ok ? 'PASS' : 'FAIL'}\t${what}\n`)
  return ok
}
// Hard precondition: later steps make no sense without it.
export function must(ok, what) {
  if (!check(ok, what)) throw new Error(`precondition failed: ${what}`)
}

export const evidence = (name) => path.join(runDir, name)
export const shot = (page, name) => page.screenshot({ path: evidence(`${name}.png`), fullPage: true })

// Fake camera/mic and screen capture so publishing works headless; autoplay so players start.
export async function launchBrowser() {
  return chromium.launch({
    executablePath: CHROME,
    headless: true,
    args: [
      '--use-fake-device-for-media-stream=device-count=3', // see phoneContext
      '--use-fake-ui-for-media-stream',
      '--auto-select-desktop-capture-source=Entire screen',
      '--autoplay-policy=no-user-gesture-required',
    ],
  })
}

// Every context records console errors, HTTP >= 400 and CSP violations into the evidence log. CSP violations are
// kept per context across navigations (checkout -> success -> broadcast), only for documents of this app's origin:
// third-party frames such as Toss's run under their own policy.
const cspLogs = new WeakMap()
export async function newContext(browser, label, options = {}) {
  const context = await browser.newContext({ ignoreHTTPSErrors: true, ...options })
  const violations = []
  cspLogs.set(context, violations)
  await context.exposeBinding('__verifyCsp', (_source, v) => {
    violations.push(v)
    log(`[${label} csp]`, v)
  })
  await context.addInitScript((origin) => {
    if (location.origin !== origin) return
    document.addEventListener('securitypolicyviolation', (e) => window.__verifyCsp(`${location.pathname} ${e.violatedDirective} ${e.blockedURI}`))
  }, new URL(BASE).origin)
  context.on('page', (page) => watch(page, label))
  return context
}

function watch(page, label) {
  page.on('console', (m) => m.type() === 'error' && log(`[${label} console.error]`, m.text().slice(0, 300)))
  page.on('response', (r) => r.status() >= 400 && log(`[${label} http ${r.status()}]`, r.request().method(), r.url()))
  page.on('pageerror', (e) => log(`[${label} pageerror]`, e.message))
}

export const cspViolations = (page) => [...(cspLogs.get(page.context()) ?? [])]

// Same-origin JSON call from inside the page, so the session cookie and the CSRF header apply.
export function api(page, method, url, body, headers = {}) {
  return page.evaluate(
    async ({ method, url, body, headers }) => {
      const r = await fetch(url, {
        method,
        headers: { 'Content-Type': 'application/json', 'X-Requested-With': 'live-platform', ...headers },
        body: body === undefined ? undefined : JSON.stringify(body),
      })
      const text = await r.text()
      let json = null
      try { json = JSON.parse(text) } catch { /* not JSON */ }
      return { status: r.status, json, text: text.slice(0, 500) }
    },
    { method, url, body, headers },
  )
}

// Accounts are kept per role in $VERIFY_HOME/accounts.json and reused: sign-up allows 5 per client per 2 minutes,
// so running every journey back to back with fresh accounts hits 429. A role whose login fails (the database was
// reset) signs up again. Any broadcast the account left LIVE or INTERRUPTED is stopped first.
const ACCOUNTS = path.join(home, 'accounts.json')
export async function account(page, role) {
  const saved = existsSync(ACCOUNTS) ? JSON.parse(readFileSync(ACCOUNTS, 'utf8')) : {}
  await page.goto(BASE)
  let creds = saved[role]
  if (creds && (await api(page, 'POST', '/api/auth/login', creds)).status !== 200) creds = undefined
  if (!creds) {
    creds = { username: `v${role.replace(/[^a-z0-9]/g, '').slice(0, 6)}${randomBytes(4).toString('hex')}`, password: randomBytes(12).toString('base64url') }
    const r = await api(page, 'POST', '/api/auth/signup', creds)
    must(r.status < 300, `sign up ${creds.username} (${r.status}${r.status === 429 ? ': sign-up limit, wait 2 minutes' : ''})`)
    writeFileSync(ACCOUNTS, JSON.stringify({ ...saved, [role]: creds }, null, 2))
  }
  const me = (await api(page, 'GET', '/api/auth/me')).json
  must(me?.id, `${role} signed in as ${creds.username}`)
  const live = (await api(page, 'GET', '/api/broadcasts?live=true')).json?.items ?? []
  for (const b of live.filter((b) => b.ownerId === me.id)) {
    log(`stopping ${b.id}, left ${b.status} by an earlier run`)
    await api(page, 'POST', `/api/broadcasts/${b.id}/stop`)
  }
  return { ...creds, id: me.id }
}
export async function createBroadcast(page, title) {
  const r = await api(page, 'POST', '/api/broadcasts', { title, description: `verify-live-platform ${RUN_ID}` })
  must(r.status === 201, `create broadcast (${r.status} ${r.status === 201 ? '' : r.text})`)
  return r.json.id
}

export async function broadcastStatus(page, id) {
  return (await api(page, 'GET', `/api/broadcasts/${id}`)).json?.status
}

export async function waitStatus(page, id, want, ms = 40_000) {
  const end = Date.now() + ms
  let last
  while (Date.now() < end) {
    last = await broadcastStatus(page, id)
    if (last === want) return true
    await page.waitForTimeout(500)
  }
  log(`status stayed ${last}, wanted ${want}`)
  return false
}

// Owner's page: press 방송 시작 and wait for the publisher to report a running WHIP session.
export async function startPublishing(page, id) {
  await page.goto(`${BASE}/broadcasts/${id}`)
  await page.getByRole('button', { name: '방송 시작' }).click()
  await page.locator('.publish-state').getByText(/^송출 중/).waitFor({ timeout: 30_000 })
  return waitStatus(page, id, 'LIVE', 20_000)
}

// The live player: section[aria-label=시청] carries data-source=ll-hls|whep once a source plays.
export async function assertPlaying(page, kind, label) {
  const player = page.locator(`section.player[data-source="${kind}"]`)
  await player.waitFor({ timeout: 30_000 })
  await page.waitForFunction(() => document.querySelector('section.player video')?.currentTime > 0, null, { timeout: 30_000 })
  const t0 = await page.$eval('section.player video', (v) => v.currentTime)
  await page.waitForTimeout(3000)
  const v = await page.$eval('section.player video', (v) => ({ t: v.currentTime, w: v.videoWidth, h: v.videoHeight }))
  return check(v.t > t0 && v.w > 0, `${label}: ${kind} plays (${t0.toFixed(1)}s -> ${v.t.toFixed(1)}s, ${v.w}x${v.h})`)
}

// Best effort stop so a failed run does not leave a LIVE broadcast behind.
export async function stopBroadcast(page, id) {
  return api(page, 'POST', `/api/broadcasts/${id}/stop`).catch(() => null)
}

// Phone emulation: Playwright device (UA, viewport, isMobile, touch). Desktop Chrome has getDisplayMedia and phones
// do not, so it is removed before page scripts run.
// Chrome's fake cameras report no facingMode, so a camera switch takes the app's fallback: the next videoinput by
// deviceId. Chrome is launched with three fake cameras, and fake_device_1 is hidden: it delivers frames that WebRTC
// never encodes (0 frames encoded, measured with Chrome 156), so a viewer would freeze on it. The switch then goes
// fake_device_0 <-> fake_device_2, and both encode.
export async function phoneContext(browser, label, deviceName = 'Pixel 7') {
  const context = await newContext(browser, label, { ...devices[deviceName] })
  await context.addInitScript(() => {
    if (typeof MediaDevices === 'undefined') return // about:blank and other insecure documents
    delete MediaDevices.prototype.getDisplayMedia
    const enumerate = MediaDevices.prototype.enumerateDevices
    MediaDevices.prototype.enumerateDevices = async function () {
      return (await enumerate.call(this)).filter((d) => d.label !== 'fake_device_1')
    }
  })
  return context
}

// Page Visibility override: the publisher reacts to visibilitychange the way a phone app switch triggers it.
export function setHidden(page, hidden) {
  return page.evaluate((hidden) => {
    Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => (hidden ? 'hidden' : 'visible') })
    Object.defineProperty(document, 'hidden', { configurable: true, get: () => hidden })
    document.dispatchEvent(new Event('visibilitychange'))
  }, hidden)
}

export async function noCspViolations(pages) {
  for (const [label, page] of pages) {
    const v = cspViolations(page)
    check(v.length === 0, `${label}: no CSP violations${v.length ? ` (${v.join('; ')})` : ''}`)
  }
}

export function summary() {
  const failed = results.filter((r) => !r.ok)
  log(failed.length === 0 ? `VERIFY OK (${results.length} checks)` : `VERIFY FAILED (${failed.length}/${results.length})`)
  for (const f of failed) log('  failed:', f.what)
  return failed.length === 0
}
