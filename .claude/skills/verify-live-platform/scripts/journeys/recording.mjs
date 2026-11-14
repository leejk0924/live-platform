// Feature: recording (features/recording.md). The owner goes live for about 10 s and ends the broadcast. The
// recording turns READY with a file in recording storage. An anonymous viewer finds it under /recordings, plays
// it and seeks. A forged file URL gets 403. After the owner deletes the broadcast, the replay and file URLs get 404.
import { execFileSync } from 'node:child_process'
import { copyFileSync, readdirSync, rmSync, statSync } from 'node:fs'
import path from 'node:path'
import {
  BASE, REPO, RUN_ID, log, check, must, shot, evidence, launchBrowser, newContext, account, createBroadcast,
  startPublishing, waitStatus, stopBroadcast, api, noCspViolations, summary,
} from '../lib.mjs'

const browser = await launchBrowser()
let ownerPage, id

function findFiles(dir, name) {
  const out = []
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, entry.name)
    if (entry.isDirectory()) out.push(...findFiles(p, name))
    else if (entry.name === name) out.push(p)
  }
  return out
}

try {
  const owner = await newContext(browser, 'owner')
  ownerPage = await owner.newPage()
  const o = await account(ownerPage, 'recorder', { streamer: true })
  const title = `verify recording ${RUN_ID}`
  id = await createBroadcast(ownerPage, title)
  log('owner', o.username, 'broadcast', id)

  must(await startPublishing(ownerPage, id), 'owner publishes and the broadcast turns LIVE')
  await ownerPage.waitForTimeout(10_000)
  await ownerPage.getByRole('button', { name: '방송 종료' }).click()
  must(await waitStatus(ownerPage, id, 'ENDED', 15_000), 'broadcast turns ENDED after 방송 종료')

  // The segment closes at the end of the broadcast; the hook registers it and the move to storage runs in the background.
  let recording
  const end = Date.now() + 60_000
  while (Date.now() < end) {
    const r = await api(ownerPage, 'GET', `/api/recordings?broadcastId=${id}`)
    const items = r.json?.items ?? r.json ?? []
    recording = items[0]
    if (items.length && !['PENDING', 'UPLOADING'].includes(recording.status)) break
    await ownerPage.waitForTimeout(1000)
  }
  must(recording?.status === 'READY', `recording turns READY (${recording?.status ?? 'none'})`)
  check(recording.durationMs >= 8000 && recording.sizeBytes > 0, `recording has ${recording.durationMs} ms and ${recording.sizeBytes} bytes`)
  log('recording', recording.id)

  // Side effect: the file sits in the recording_store volume. The api image is distroless, so copy it out.
  const store = evidence('store')
  execFileSync('docker', ['compose', 'cp', 'api:/recording-store', store], { cwd: REPO, stdio: 'pipe' })
  const files = findFiles(store, `${recording.id}.mp4`)
  check(files.length === 1 && statSync(files[0]).size === Number(recording.sizeBytes),
    `storage holds ${files.map((f) => path.relative(store, f).replaceAll('\\', '/')).join(', ') || 'no file'} with the reported size`)
  // Keep this run's file as evidence, not the whole volume.
  if (files.length === 1) copyFileSync(files[0], evidence('recording.mp4'))
  rmSync(store, { recursive: true, force: true })

  // An anonymous viewer finds it in the recordings list and plays it.
  const viewer = await newContext(browser, 'viewer')
  const viewerPage = await viewer.newPage()
  await viewerPage.goto(`${BASE}/recordings`)
  const feed = viewerPage.locator('section.feed[aria-label="녹화 목록"]')
  await feed.getByText(title).waitFor({ timeout: 15_000 })
  check(true, '/recordings lists it for an anonymous viewer')
  await shot(viewerPage, '1-recordings-list')
  await feed.getByText(title).click()
  await viewerPage.waitForURL(`**/recordings/${recording.id}`)
  const video = viewerPage.locator('section.player[aria-label="다시보기"] video')
  await viewerPage.waitForFunction(() => document.querySelector('section.player[aria-label="다시보기"] video')?.readyState >= 1, null, { timeout: 15_000 })
  const played = await video.evaluate(async (v) => {
    v.muted = true
    await v.play()
    const t0 = v.currentTime
    await new Promise((r) => setTimeout(r, 2000))
    const t1 = v.currentTime
    v.currentTime = 4
    await new Promise((r) => (v.onseeked = r))
    await new Promise((r) => setTimeout(r, 1500))
    return { t0, t1, after: v.currentTime, duration: v.duration, w: v.videoWidth }
  })
  check(played.t1 > played.t0 && played.w > 0, `replay plays (${played.t0.toFixed(1)}s -> ${played.t1.toFixed(1)}s of ${played.duration.toFixed(1)}s)`)
  check(played.after > 4, `seek to 4s continues from there (${played.after.toFixed(1)}s)`)
  await shot(viewerPage, '2-replay-playing')

  // Signed file URL: the real one serves, a forged signature is refused.
  const replay = (await api(viewerPage, 'GET', `/api/recordings/${recording.id}/replay`)).json
  must(replay?.url, 'replay endpoint returns a file URL')
  const fileUrl = new URL(replay.url, BASE)
  const fetchStatus = (url) => viewerPage.evaluate((u) => fetch(u, { headers: { Range: 'bytes=0-15' } }).then((r) => r.status), url)
  const realStatus = await fetchStatus(fileUrl.href)
  check(realStatus === 206, `signed file URL serves a range (${realStatus})`)
  const forged = new URL(fileUrl)
  forged.searchParams.set('signature', forged.searchParams.get('signature').replace(/^./, (c) => (c === 'a' ? 'b' : 'a')))
  const forgedStatus = await fetchStatus(forged.href)
  check(forgedStatus === 403, `forged signature gets ${forgedStatus}`)

  // Deleting the broadcast takes its recording with it.
  const del = await api(ownerPage, 'DELETE', `/api/broadcasts/${id}`)
  check(del.status === 204, `owner deletes the ENDED broadcast (${del.status})`)
  const after = await api(viewerPage, 'GET', `/api/recordings/${recording.id}/replay`)
  check(after.status === 404, `replay endpoint after delete gets ${after.status}`)
  const issued = await fetchStatus(fileUrl.href)
  check(issued === 404, `already-issued file URL after delete gets ${issued}`)
  id = undefined

  await noCspViolations([['owner', ownerPage], ['viewer', viewerPage]])
} catch (e) {
  check(false, `journey aborted: ${e.message.split('\n')[0]}`)
  if (ownerPage && id) await stopBroadcast(ownerPage, id)
} finally {
  await browser.close()
}
process.exit(summary() ? 0 : 1)
