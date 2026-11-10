// Feature: publish (features/publish.md). An emulated Android phone goes live with the camera, switches cameras
// before and during the broadcast while a desktop viewer keeps playing, fits the screen in both orientations,
// pauses in the background and resumes within the 30 s grace, then ends past it. A desktop publisher keeps
// publishing while hidden.
import {
  BASE, RUN_ID, log, check, must, shot, launchBrowser, newContext, phoneContext, account, createBroadcast,
  startPublishing, assertPlaying, broadcastStatus, waitStatus, stopBroadcast, setHidden, noCspViolations, summary,
} from '../lib.mjs'

const browser = await launchBrowser()
let phonePage, desktopPage, id, desktopId
const publisher = (page) => page.locator('section.publisher[aria-label="송출"]')
const state = (page) => page.locator('.publish-state').innerText()
const overflow = (page) => page.evaluate(() => document.documentElement.scrollWidth - innerWidth)

// The viewer's video keeps moving: currentTime advances over 3 s.
async function viewerAdvances(page, label) {
  const t0 = await page.$eval('section.player video', (v) => v.currentTime)
  await page.waitForTimeout(3000)
  const t1 = await page.$eval('section.player video', (v) => v.currentTime)
  return check(t1 > t0, `${label}: viewer keeps playing (${t0.toFixed(1)}s -> ${t1.toFixed(1)}s)`)
}

try {
  const phone = await phoneContext(browser, 'phone', 'Pixel 7')
  phonePage = await phone.newPage()
  const o = await account(phonePage, 'phone')
  id = await createBroadcast(phonePage, `verify publish ${RUN_ID}`)
  log('owner', o.username, 'broadcast', id)

  await phonePage.goto(`${BASE}/broadcasts/${id}`)
  await publisher(phonePage).waitFor()
  check((await phonePage.locator('.source-choice').count()) === 0, 'phone has no screen/camera choice (camera only)')
  check((await overflow(phonePage)) === 0, 'portrait 412 px: no horizontal scroll')
  const heights = await publisher(phonePage).locator('button').evaluateAll((bs) => bs.map((b) => [b.textContent.trim(), Math.round(b.getBoundingClientRect().height)]))
  check(heights.length > 0 && heights.every(([, h]) => h >= 44), `publisher buttons at least 44 px tall ${JSON.stringify(heights)}`)

  const facing = phonePage.locator('.facing')
  const before = await facing.innerText()
  await phonePage.getByRole('button', { name: /카메라 전환/ }).click()
  const afterIdle = await facing.innerText()
  check(afterIdle !== before, `camera switch before going live: ${before} -> ${afterIdle}`)
  await shot(phonePage, '1-phone-idle')

  await phonePage.getByRole('button', { name: '방송 시작' }).click()
  await phonePage.waitForFunction(() => document.querySelector('.publish-state')?.textContent.includes('kbps'), null, { timeout: 30_000 })
  must(await waitStatus(phonePage, id, 'LIVE', 20_000), `phone publishes: "${await state(phonePage)}", status LIVE`)
  await shot(phonePage, '2-phone-publishing')

  const viewer = await newContext(browser, 'viewer')
  const viewerPage = await viewer.newPage()
  await viewerPage.goto(`${BASE}/broadcasts/${id}`)
  await assertPlaying(viewerPage, 'll-hls', 'desktop viewer')

  // Switch cameras twice while live: the broadcast stays LIVE and the viewer keeps playing.
  let was = await facing.innerText()
  for (const n of [1, 2]) {
    await phonePage.getByRole('button', { name: /카메라 전환/ }).click()
    await phonePage.waitForTimeout(4000)
    const err = await publisher(phonePage).locator('.error').allInnerTexts()
    const now = await facing.innerText()
    check(err.length === 0 && now !== was && (await state(phonePage)).startsWith('송출 중'),
      `live switch ${n}: ${was} -> ${now}, "${await state(phonePage)}"${err.length ? ` error ${JSON.stringify(err)}` : ''}`)
    was = now
    check((await waitStatus(phonePage, id, 'LIVE', 2000)), `live switch ${n}: status stays LIVE`)
    await viewerAdvances(viewerPage, `live switch ${n}`)
  }
  await shot(viewerPage, '3-viewer-after-switch')

  // Rotation: layout only. Rotating a real phone changes the stream size and can freeze LL-HLS (no CVO).
  await phonePage.setViewportSize({ width: 915, height: 412 })
  await phonePage.waitForTimeout(500)
  check((await overflow(phonePage)) === 0, 'landscape 915 px: no horizontal scroll')
  await shot(phonePage, '4-phone-landscape')
  await phonePage.setViewportSize({ width: 412, height: 915 })

  // Background within the grace: pause, INTERRUPTED, come back, LIVE again.
  await setHidden(phonePage, true)
  await publisher(phonePage).getByText('송출을 멈췄습니다').waitFor({ timeout: 5000 })
  check(true, 'hidden phone shows the paused notice')
  check(await waitStatus(phonePage, id, 'INTERRUPTED', 20_000), 'hidden phone: status turns INTERRUPTED')
  await shot(phonePage, '5-phone-paused')
  await setHidden(phonePage, false)
  await phonePage.waitForFunction(() => document.querySelector('.publish-state')?.textContent.startsWith('송출 중'), null, { timeout: 20_000 })
  check(await waitStatus(phonePage, id, 'LIVE', 20_000), 'back within 30 s: publishing resumes and status is LIVE')

  // Background past the grace: ENDED, and the camera stays off after coming back.
  await setHidden(phonePage, true)
  check(await waitStatus(phonePage, id, 'INTERRUPTED', 20_000), 'hidden again: INTERRUPTED')
  check(await waitStatus(phonePage, id, 'ENDED', 60_000), 'past the 30 s grace: ENDED')
  await setHidden(phonePage, false)
  await phonePage.waitForTimeout(3000)
  const camera = await phonePage.evaluate(() => {
    const s = document.querySelector('.preview')?.srcObject
    return s ? s.getVideoTracks().filter((t) => t.readyState === 'live').length : 0
  })
  check(camera === 0 && !(await phonePage.locator('.publish-state').count() && (await state(phonePage)).startsWith('송출 중')),
    `back after the end: camera stays off (${camera} live tracks)`)
  await shot(phonePage, '6-phone-ended')
  id = undefined

  // A desktop publisher does not pause when its tab is hidden.
  const desktop = await newContext(browser, 'desktop')
  desktopPage = await desktop.newPage()
  await account(desktopPage, 'phone')
  desktopId = await createBroadcast(desktopPage, `verify publish desktop ${RUN_ID}`)
  must(await startPublishing(desktopPage, desktopId), 'desktop publishes the screen and turns LIVE')
  await setHidden(desktopPage, true)
  await desktopPage.waitForTimeout(10_000)
  check((await broadcastStatus(desktopPage, desktopId)) === 'LIVE' && (await state(desktopPage)).startsWith('송출 중'),
    `hidden desktop keeps publishing after 10 s: "${await state(desktopPage)}"`)
  await setHidden(desktopPage, false)
  await desktopPage.getByRole('button', { name: '방송 종료' }).click()
  check(await waitStatus(desktopPage, desktopId, 'ENDED', 15_000), 'desktop 방송 종료: ENDED')
  desktopId = undefined

  await noCspViolations([['phone', phonePage], ['viewer', viewerPage], ['desktop', desktopPage]])
} catch (e) {
  check(false, `journey aborted: ${e.message.split('\n')[0]}`)
  if (phonePage && id) await stopBroadcast(phonePage, id)
  if (desktopPage && desktopId) await stopBroadcast(desktopPage, desktopId)
} finally {
  await browser.close()
}
process.exit(summary() ? 0 : 1)

