// Feature: comments (features/comments.md). The author posts a script tag as a comment on its broadcast page and a
// second signed-in reader sees it live as text. A second post inside a second is refused with a wait time. The
// author edits and deletes, and the reader sees both live. The reader cannot edit the author's comment (403).
// A reader that reconnects gets what it missed.
import {
  BASE, RUN_ID, log, check, must, shot, launchBrowser, newContext, account, createBroadcast, api, noCspViolations, summary,
} from '../lib.mjs'

const browser = await launchBrowser()
let authorPage, id
const comments = (page) => page.locator('section.comments[aria-label="댓글"]')
const bodies = (page) => comments(page).locator('.comment-body').allInnerTexts()

async function post(page, text) {
  await comments(page).getByLabel('댓글', { exact: true }).fill(text)
  await comments(page).getByRole('button', { name: '등록' }).click()
}

// Waits for a predicate over the reader's visible comment bodies and returns how long it took.
async function seen(page, what, predicate, ms = 5000) {
  const start = Date.now()
  while (Date.now() - start < ms) {
    if (predicate(await bodies(page))) return check(true, `${what} (${((Date.now() - start) / 1000).toFixed(1)}s, no reload)`)
    await page.waitForTimeout(100)
  }
  return check(false, `${what}: not seen within ${ms / 1000}s, reader shows ${JSON.stringify(await bodies(page))}`)
}

try {
  const author = await newContext(browser, 'author')
  authorPage = await author.newPage()
  const a = await account(authorPage, 'author')
  id = await createBroadcast(authorPage, `verify comments ${RUN_ID}`)
  log('author', a.username, 'broadcast', id)

  const reader = await newContext(browser, 'reader')
  let readerPage = await reader.newPage()
  const r = await account(readerPage, 'reader')
  log('reader', r.username)

  await authorPage.goto(`${BASE}/broadcasts/${id}`)
  await readerPage.goto(`${BASE}/broadcasts/${id}`)
  await comments(readerPage).getByText('아직 댓글이 없습니다.').waitFor({ timeout: 15_000 })

  const xss = `<script>alert(1)</script> ${RUN_ID}`
  await post(authorPage, xss)
  await comments(authorPage).locator('.comment-body', { hasText: xss }).waitFor({ timeout: 10_000 })
  check(true, 'author sees its comment in .comment-list')
  await seen(readerPage, 'reader gets the comment live', (b) => b.some((t) => t.includes(xss)), 3000)
  for (const [label, page] of [['author', authorPage], ['reader', readerPage]]) {
    const ran = await page.evaluate(() => [...document.scripts].some((s) => s.textContent.includes('alert(1)')) || document.querySelector('.comment-body script') !== null)
    check(!ran, `${label}: the script tag stays text (no <script> element added)`)
  }
  await shot(readerPage, '1-reader-live-comment')

  // Rate limit: one comment per second per user. Wait out the first comment's second, then post twice.
  await authorPage.waitForTimeout(1100)
  await post(authorPage, `first ${RUN_ID}`)
  await comments(authorPage).locator('.comment-body', { hasText: `first ${RUN_ID}` }).waitFor({ timeout: 10_000 })
  await post(authorPage, `too soon ${RUN_ID}`)
  const limited = comments(authorPage).locator('.comment-form [role=alert]')
  await limited.waitFor({ timeout: 5000 }).catch(() => {})
  const limitText = (await limited.count()) ? await limited.innerText() : ''
  check(/초 후/.test(limitText), `second comment within 1s is refused: "${limitText}"`)
  await shot(authorPage, '2-author-rate-limited')
  await authorPage.waitForTimeout(1100)

  // Edit: the author's own comment shows 수정, the reader sees the new text and (수정됨).
  const mine = comments(authorPage).locator('li.comment', { hasText: `first ${RUN_ID}` })
  check((await comments(readerPage).locator('li.comment', { hasText: `first ${RUN_ID}` }).getByRole('button', { name: '수정' }).count()) === 0,
    'reader has no 수정 button on the author\'s comment')
  await mine.getByRole('button', { name: '수정' }).click()
  // Editing turns the text into an input, so the row no longer matches by text: the open edit form is the only one.
  const editForm = comments(authorPage).locator('form.comment-edit')
  await editForm.getByLabel('댓글 수정').fill(`edited ${RUN_ID}`)
  await editForm.getByRole('button', { name: '저장' }).click()
  await seen(readerPage, 'reader gets the edit live', (b) => b.some((t) => t.includes(`edited ${RUN_ID}`) && t.includes('(수정됨)')))
  await shot(readerPage, '3-reader-edited')

  // Someone else's comment: 403 NOT_COMMENT_AUTHOR from the API, whatever the UI hides.
  const list = (await api(readerPage, 'GET', `/api/broadcasts/${id}/comments`)).json
  const target = (list?.items ?? list ?? []).find((c) => c.body === `edited ${RUN_ID}`)
  must(target, 'reader finds the edited comment in the list API')
  const forged = await api(readerPage, 'PUT', `/api/broadcasts/${id}/comments/${target.id}`, { body: 'hijack' }, { 'If-Match': `"${target.version}"` })
  check(forged.status === 403 && forged.json?.code === 'NOT_COMMENT_AUTHOR', `reader editing it gets ${forged.status} ${forged.json?.code}`)
  const forgedDelete = await api(readerPage, 'DELETE', `/api/broadcasts/${id}/comments/${target.id}`)
  check(forgedDelete.status === 403, `reader deleting it gets ${forgedDelete.status} ${forgedDelete.json?.code}`)

  // Delete with the confirm dialog.
  authorPage.once('dialog', (d) => d.accept())
  await comments(authorPage).locator('li.comment', { hasText: `edited ${RUN_ID}` }).getByRole('button', { name: '삭제' }).click()
  await seen(readerPage, 'reader sees the delete live', (b) => !b.some((t) => t.includes(`edited ${RUN_ID}`)))

  // Resume: what was posted while the reader was away shows up when it comes back.
  await readerPage.close()
  await post(authorPage, `while away ${RUN_ID}`)
  await comments(authorPage).locator('.comment-body', { hasText: `while away ${RUN_ID}` }).waitFor({ timeout: 10_000 })
  readerPage = await reader.newPage()
  await readerPage.goto(`${BASE}/broadcasts/${id}`)
  await seen(readerPage, 'reopened reader shows the comment posted while it was away', (b) => b.some((t) => t.includes(`while away ${RUN_ID}`)), 10_000)
  await shot(readerPage, '4-reader-reopened')

  await noCspViolations([['author', authorPage], ['reader', readerPage]])
} catch (e) {
  check(false, `journey aborted: ${e.message.split('\n')[0]}`)
} finally {
  // READY broadcasts can be deleted: keep the local database from filling up with journey rows.
  if (authorPage && id) log('delete broadcast', (await api(authorPage, 'DELETE', `/api/broadcasts/${id}`).catch(() => ({}))).status)
  await browser.close()
}
process.exit(summary() ? 0 : 1)
