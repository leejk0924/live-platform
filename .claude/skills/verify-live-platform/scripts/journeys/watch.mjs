// Feature: watch (features/watch.md). Owner goes live from the desktop publisher, an anonymous viewer finds it
// under 방송 중 and plays LL-HLS, switches to WebRTC (WHEP), an emulated iPhone plays WHEP, the owner ends it
// and every viewer gets the end notice while home moves it to 지난·예정 방송.
import { execFileSync } from 'node:child_process'
import {
  BASE, RUN_ID, log, check, must, shot, launchBrowser, newContext, devices, account, createBroadcast,
  startPublishing, assertPlaying, waitStatus, stopBroadcast, noCspViolations, summary,
} from '../lib.mjs'

const browser = await launchBrowser()
let ownerPage, id
try {
  const owner = await newContext(browser, 'owner')
  ownerPage = await owner.newPage()
  const { username } = await account(ownerPage, 'caster')
  const title = `verify watch ${RUN_ID}`
  id = await createBroadcast(ownerPage, title)
  log('owner', username, 'broadcast', id)

  must(await startPublishing(ownerPage, id), 'owner publishes and the broadcast turns LIVE')
  await shot(ownerPage, '1-owner-publishing')

  // Anonymous API: playback sources need no cookie.
  const playback = execFileSync('curl', ['-ks', '-w', '\n%{http_code}', `${BASE}/api/broadcasts/${id}/playback`]).toString().trim()
  const [bodyText, code] = [playback.slice(0, playback.lastIndexOf('\n')), playback.slice(playback.lastIndexOf('\n') + 1)]
  const kinds = code === '200' ? JSON.parse(bodyText).map((s) => s.kind).join(',') : ''
  check(code === '200' && kinds === 'll-hls,whep', `GET /playback without cookie is 200 [${kinds}] (${code})`)

  const viewer = await newContext(browser, 'viewer')
  must((await viewer.cookies()).length === 0, 'viewer context has no cookies')
  const viewerPage = await viewer.newPage()
  await viewerPage.goto(BASE)
  const liveFeed = viewerPage.locator('section.feed[aria-label="방송 중"]')
  await liveFeed.getByText(title).waitFor({ timeout: 15_000 })
  check(true, 'home lists the broadcast under 방송 중')
  await shot(viewerPage, '2-home-live')

  await liveFeed.getByText(title).click()
  await assertPlaying(viewerPage, 'll-hls', 'anonymous viewer default')
  check(await viewerPage.getByRole('button', { name: '소리 켜기' }).isVisible(), 'unmute button shown while muted')
  await shot(viewerPage, '3-viewer-llhls')

  await viewerPage.getByLabel('저지연(WebRTC)').check()
  await assertPlaying(viewerPage, 'whep', 'viewer low latency')
  await shot(viewerPage, '4-viewer-whep')

  const iphone = await newContext(browser, 'iphone', { ...devices['iPhone 15'] })
  const iphonePage = await iphone.newPage()
  await iphonePage.goto(`${BASE}/broadcasts/${id}`)
  await assertPlaying(iphonePage, 'whep', 'iPhone emulation picks WHEP')
  check((await iphonePage.getByLabel('저지연(WebRTC)').count()) === 0, 'iPhone has no low-latency toggle')
  await shot(iphonePage, '5-iphone-whep')

  await ownerPage.getByRole('button', { name: '방송 종료' }).click()
  const stoppedAt = Date.now()
  check(await waitStatus(ownerPage, id, 'ENDED', 15_000), 'broadcast turns ENDED after 방송 종료')
  await viewerPage.getByText('종료된 방송입니다.').waitFor({ timeout: 15_000 })
  check(true, `viewer sees the end notice ${((Date.now() - stoppedAt) / 1000).toFixed(1)}s after stop`)
  check((await viewerPage.locator('video').count()) === 0, 'viewer has no <video> after the end')
  await iphonePage.getByText('종료된 방송입니다.').waitFor({ timeout: 15_000 })
  check(true, 'iPhone viewer sees the end notice')
  await shot(viewerPage, '6-viewer-ended')

  await viewerPage.goto(BASE)
  await viewerPage.locator('section.feed[aria-label="지난·예정 방송"]').getByText(title).waitFor({ timeout: 15_000 })
  check((await liveFeed.getByText(title).count()) === 0, 'home moves it to 지난·예정 방송')
  await shot(viewerPage, '7-home-ended')

  await noCspViolations([['owner', ownerPage], ['viewer', viewerPage], ['iphone', iphonePage]])
} catch (e) {
  check(false, `journey aborted: ${e.message.split('\n')[0]}`)
  if (ownerPage && id) await stopBroadcast(ownerPage, id)
} finally {
  await browser.close()
}
process.exit(summary() ? 0 : 1)
