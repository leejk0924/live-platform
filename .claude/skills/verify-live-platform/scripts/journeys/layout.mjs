// Feature: layout (features/layout.md). Every screen of the SPA, at desktop 1280x800 and phones 390x844 and 360x780,
// in the dark and light color schemes: a full-page screenshot, no horizontal scroll, 44x44 px touch targets on the
// phones, no browser-default links or form controls, and WCAG text contrast against the background actually behind
// the text.
import {
  BASE, RUN_ID, log, check, must, shot, launchBrowser, newContext, phoneContext, account, createBroadcast,
  startPublishing, waitStatus, stopBroadcast, setHidden, api, noCspViolations, summary,
} from '../lib.mjs'

const VIEWPORTS = [
  { name: 'desktop', size: { width: 1280, height: 800 } },
  { name: 'phone', size: { width: 390, height: 844 }, phone: true },
  { name: 'phone360', size: { width: 360, height: 780 }, phone: true },
]
const SCHEMES = ['dark', 'light']
const WORST = 6

// Runs in the page. Returns every offender per rule; the journey turns each rule into one check.
function audit({ scheme, phone }) {
  const canvas = document.createElement('canvas')
  canvas.width = canvas.height = 1
  const ctx = canvas.getContext('2d', { willReadFrequently: true })
  const cache = new Map()
  // Any CSS color syntax (rgb, color(srgb), oklch, named) to straight rgba through the canvas.
  const rgba = (c) => {
    if (cache.has(c)) return cache.get(c)
    ctx.clearRect(0, 0, 1, 1)
    ctx.fillStyle = 'rgba(0, 0, 0, 0)'
    ctx.fillStyle = c
    ctx.fillRect(0, 0, 1, 1)
    const [r, g, b, a] = ctx.getImageData(0, 0, 1, 1).data
    const v = { r, g, b, a: a / 255 }
    cache.set(c, v)
    return v
  }
  const over = (top, under) => ({
    r: top.r * top.a + under.r * (1 - top.a),
    g: top.g * top.a + under.g * (1 - top.a),
    b: top.b * top.a + under.b * (1 - top.a),
    a: 1,
  })
  const lum = ({ r, g, b }) => {
    const ch = (v) => ((v /= 255) <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4)
    return 0.2126 * ch(r) + 0.7152 * ch(g) + 0.0722 * ch(b)
  }
  const ratio = (a, b) => {
    const [x, y] = [lum(a), lum(b)].sort((m, n) => n - m)
    return (x + 0.05) / (y + 0.05)
  }
  const visible = (el) => {
    if (!el.checkVisibility({ opacityProperty: true, visibilityProperty: true })) return false
    const r = el.getBoundingClientRect()
    return r.width > 0 && r.height > 0
  }
  const name = (el) => {
    const cls = [...el.classList].slice(0, 2).map((c) => `.${c}`).join('')
    const text = (el.innerText || el.value || el.getAttribute('aria-label') || '').trim().replace(/\s+/g, ' ').slice(0, 24)
    return `${el.tagName.toLowerCase()}${cls}${text ? ` "${text}"` : ''}`
  }
  // A link inside a sentence is exempt from the target size and underline rules (WCAG 2.5.8 inline exception).
  const inProse = (a) => {
    const p = a.closest('p:not(.links)')
    return p !== null && p.textContent.trim().length > a.textContent.trim().length
  }

  const result = { overflow: document.documentElement.scrollWidth - innerWidth, small: [], defaults: [], contrast: [], gradient: 0 }

  if (phone) {
    for (const el of document.querySelectorAll('button, a[href], input:not([type=hidden]), select, textarea, [role=button], summary')) {
      const target = el.matches('input[type=checkbox], input[type=radio]') ? (el.closest('label') ?? el) : el
      if (!visible(target) || (el.matches('a') && inProse(el))) continue
      const { width, height } = target.getBoundingClientRect()
      if (Math.min(width, height) < 43.5) result.small.push(`${name(target)} ${width.toFixed(0)}x${height.toFixed(0)}px`)
    }
  }

  // References styled by the user agent alone: a shadow root keeps the page's CSS out but inherits color-scheme.
  const host = document.createElement('div')
  document.body.append(host)
  const root = host.attachShadow({ mode: 'open' })
  const ref = {}
  for (const [key, tag, type] of [['button', 'button'], ['input', 'input', 'text'], ['select', 'select'], ['textarea', 'textarea'], ['a', 'a']]) {
    const el = document.createElement(tag)
    if (type) el.type = type
    if (tag === 'a') el.href = '#'
    el.textContent = tag === 'select' ? '' : 'x'
    root.append(el)
    const cs = getComputedStyle(el)
    ref[key] = { bg: cs.backgroundColor, border: `${cs.borderTopStyle} ${cs.borderTopColor}`, color: cs.color }
  }
  host.remove()
  const controls = 'button, input:not([type=hidden]):not([type=checkbox]):not([type=radio]):not([type=range]), select, textarea'
  for (const el of document.querySelectorAll(`${controls}, a[href]`)) {
    if (!visible(el)) continue
    const cs = getComputedStyle(el)
    if (el.matches('a')) {
      if (cs.color === ref.a.color) result.defaults.push(`${name(el)} has the browser link color`)
      else if (cs.textDecorationLine.includes('underline') && !inProse(el)) result.defaults.push(`${name(el)} is underlined outside prose`)
      continue
    }
    const r = ref[el.tagName === 'INPUT' ? 'input' : el.tagName.toLowerCase()]
    if (cs.backgroundColor === r.bg && `${cs.borderTopStyle} ${cs.borderTopColor}` === r.border) result.defaults.push(`${name(el)} has the browser's own background and border`)
  }

  const rootCs = getComputedStyle(document.documentElement)
  const canvasColor = scheme === 'dark' && rootCs.colorScheme.includes('dark') ? rgba('rgb(18, 18, 18)') : rgba('white')
  const STOP = /(rgba?\([^)]*\)|color\([^)]*\)|oklch\([^)]*\)|oklab\([^)]*\)|hsla?\([^)]*\))/g
  // Layers from the text down to the first opaque one; a gradient contributes each of its stops.
  const backgrounds = (el) => {
    const layers = []
    let opaque = false
    for (let n = el; n && !opaque; n = n.parentElement) {
      const cs = getComputedStyle(n)
      if (cs.backgroundImage.includes('gradient')) {
        const stops = (cs.backgroundImage.match(STOP) ?? []).map(rgba)
        if (stops.length) {
          layers.push(stops)
          if (stops.every((s) => s.a >= 1)) opaque = true
        }
      }
      const bg = rgba(cs.backgroundColor)
      if (!opaque && bg.a > 0) {
        layers.push([bg])
        if (bg.a >= 1) opaque = true
      }
    }
    let bases = opaque ? [] : [canvasColor]
    for (const layer of layers.reverse()) {
      if (bases.length === 0) bases = layer.map((c) => ({ ...c, a: 1 }))
      else bases = layer.flatMap((c) => bases.map((b) => over(c, b)))
    }
    return { bases, gradient: layers.length > 0 && layers.some((l) => l.length > 1) }
  }
  const opacityOf = (el) => {
    let o = 1
    for (let n = el; n; n = n.parentElement) o *= Number(getComputedStyle(n).opacity)
    return o
  }

  const texts = new Set()
  const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT)
  for (let t = walker.nextNode(); t; t = walker.nextNode()) if (t.textContent.trim() && t.parentElement) texts.add(t.parentElement)
  for (const el of document.querySelectorAll('input:not([type=hidden]):not([type=checkbox]):not([type=radio]), textarea')) texts.add(el)
  for (const el of texts) {
    if (el.closest('[aria-hidden=true], option, script, style, :disabled, fieldset:disabled') || !visible(el)) continue
    const placeholder = el.matches('input, textarea') && el.value === ''
    if (el.matches('input, textarea') && !el.value && !el.placeholder) continue
    const cs = getComputedStyle(el, placeholder ? '::placeholder' : null)
    const fg = rgba(cs.color)
    const { bases, gradient } = backgrounds(el)
    if (gradient) result.gradient++
    const text = { ...fg, a: fg.a * opacityOf(el) }
    const worst = Math.min(...bases.map((b) => ratio(over(text, b), b)))
    const size = parseFloat(cs.fontSize)
    const large = size >= 24 || (size >= 18.66 && Number(cs.fontWeight) >= 700)
    const need = large ? 3 : 4.5
    if (worst < need) result.contrast.push({ what: `${name(el)}${placeholder ? ' (placeholder)' : ''}`, ratio: worst, need })
  }
  return result
}

const browser = await launchBrowser()
const pages = []
let castPage, liveId, phonePage, phoneId
try {
  const castContext = await newContext(browser, 'caster')
  castPage = await castContext.newPage()
  const caster = await account(castPage, 'lcaster')
  const viewerContext = await newContext(browser, 'viewer')
  const viewerPage = await viewerContext.newPage()
  await account(viewerPage, 'lviewer')

  const endedTitle = `verify layout ended ${RUN_ID}`
  const endedId = await createBroadcast(castPage, endedTitle)
  must(await startPublishing(castPage, endedId), 'a broadcast goes LIVE to leave a recording')
  await castPage.waitForTimeout(8000)
  await castPage.getByRole('button', { name: '방송 종료' }).click()
  must(await waitStatus(castPage, endedId, 'ENDED', 15_000), 'that broadcast turns ENDED')
  let recordingId
  const recordingDeadline = Date.now() + 60_000
  while (Date.now() < recordingDeadline && !recordingId) {
    const items = (await api(castPage, 'GET', `/api/recordings?broadcastId=${endedId}`)).json?.items ?? []
    if (items[0]?.status === 'READY') recordingId = items[0].id
    else await castPage.waitForTimeout(1000)
  }
  must(recordingId, 'its recording turns READY')

  const liveTitle = `verify layout ${RUN_ID}`
  liveId = await createBroadcast(castPage, liveTitle)
  must(await startPublishing(castPage, liveId), 'the main broadcast goes LIVE')
  check((await api(viewerPage, 'POST', `/api/broadcasts/${liveId}/comments`, { body: `안녕하세요, 레이아웃 확인 ${RUN_ID}` })).status === 201, 'viewer posts a comment for the sidebar')
  await viewerPage.waitForTimeout(1100)
  check((await api(castPage, 'POST', `/api/broadcasts/${liveId}/comments`, { body: '반갑습니다. 오늘은 화면 공유로 진행합니다.' })).status === 201, 'caster posts a comment for the sidebar')

  const readyId = await createBroadcast(castPage, `verify layout ready ${RUN_ID}`)

  // The owner already has a LIVE broadcast, so a second account publishes from a phone and goes INTERRUPTED in the
  // background for each interrupted screen, then comes back within the 30 s grace.
  phonePage = await (await phoneContext(browser, 'phone-publisher')).newPage()
  await account(phonePage, 'lphone')
  phoneId = await createBroadcast(phonePage, `verify layout interrupted ${RUN_ID}`)
  must(await startPublishing(phonePage, phoneId), 'a phone publisher goes LIVE to be interrupted')
  const interrupt = async () => {
    await setHidden(phonePage, true)
    if (!(await waitStatus(phonePage, phoneId, 'INTERRUPTED', 20_000))) throw new Error('the hidden phone publisher did not turn INTERRUPTED')
  }
  const resume = async () => {
    await setHidden(phonePage, false)
    check(await waitStatus(phonePage, phoneId, 'LIVE', 30_000), 'the phone publisher comes back LIVE within the grace')
  }

  const ready = {
    feed: (title) => (p) => p.locator('section.feed').getByText(title).first().waitFor({ timeout: 15_000 }),
    text: (text) => (p) => p.getByText(text).first().waitFor({ timeout: 15_000 }),
    css: (selector) => (p) => p.locator(selector).first().waitFor({ timeout: 30_000 }),
  }
  const screens = [
    { name: 'home', role: 'viewer', path: '/', ready: ready.feed(liveTitle) },
    { name: 'home-guest', role: 'guest', path: '/', ready: ready.feed(liveTitle) },
    { name: 'watch', role: 'viewer', path: `/broadcasts/${liveId}`, ready: ready.css('section.player[data-source] video') },
    { name: 'studio', role: 'caster', path: `/broadcasts/${liveId}`, ready: ready.css('section.publisher') },
    { name: 'studio-ready', role: 'caster', path: `/broadcasts/${readyId}`, ready: ready.css('section.publisher') },
    { name: 'interrupted', role: 'viewer', path: `/broadcasts/${phoneId}`, ready: ready.text('방송 연결이 끊겼습니다'), before: interrupt, after: resume },
    { name: 'ended', role: 'guest', path: `/broadcasts/${endedId}`, ready: ready.text('종료된 방송입니다.') },
    { name: 'broadcast-new', role: 'caster', path: '/broadcasts/new', ready: ready.css('form input[name=title]') },
    { name: 'broadcast-edit', role: 'caster', path: `/broadcasts/${liveId}/edit`, ready: ready.css('form input[name=title]') },
    { name: 'login', role: 'guest', path: '/login', ready: ready.css('form.credentials') },
    { name: 'signup', role: 'guest', path: '/signup', ready: ready.css('form.credentials') },
    { name: 'recordings', role: 'guest', path: '/recordings', ready: ready.feed(endedTitle) },
    { name: 'recording', role: 'guest', path: `/recordings/${recordingId}`, ready: ready.css('section.player[aria-label="다시보기"][data-state="ready"]') },
    { name: 'pay-checkout', role: 'viewer', path: `/pay/checkout?broadcast=${liveId}&amount=1`, ready: ready.css('.error') },
    { name: 'pay-success', role: 'viewer', path: `/pay/success?broadcast=${liveId}`, ready: ready.css('.error') },
    { name: 'pay-fail', role: 'viewer', path: `/pay/fail?broadcast=${liveId}&code=PAY_PROCESS_CANCELED`, ready: ready.text('결제를 취소했습니다.') },
    { name: 'not-found', role: 'guest', path: '/no-such-page', ready: ready.text('페이지를 찾을 수 없습니다') },
  ]
  const tally = { overflow: 0, touch: 0, defaults: 0, contrast: 0 }

  const sessions = { caster: await castContext.storageState(), viewer: await viewerContext.storageState(), guest: undefined }
  for (const { name: viewport, size, phone = false } of VIEWPORTS) {
    const contexts = {}
    for (const role of Object.keys(sessions)) {
      const label = `${role}-${viewport}`
      const options = { storageState: sessions[role], viewport: size }
      contexts[role] = phone
        ? await phoneContext(browser, label, 'Pixel 7', options)
        : await newContext(browser, label, options)
      const page = await contexts[role].newPage()
      pages.push([label, page])
      contexts[role] = page
    }
    for (const scheme of SCHEMES) {
      for (const screen of screens) {
        const page = contexts[screen.role]
        const id = `${screen.name}-${viewport}-${scheme}`
        let r
        try {
          await screen.before?.()
          await page.emulateMedia({ colorScheme: scheme })
          await page.goto(`${BASE}${screen.path}`)
          await screen.ready(page)
          await page.waitForTimeout(1000)
          await shot(page, id)
          r = await page.evaluate(audit, { scheme, phone })
        } catch (e) {
          check(false, `${id}: screen ready (${e.message.split('\n')[0]})`)
        } finally {
          await screen.after?.()
        }
        if (!r) continue
        if (!check(r.overflow <= 0, `${id}: no horizontal scroll (scrollWidth - innerWidth = ${r.overflow})`)) tally.overflow++
        if (phone && !check(r.small.length === 0, `${id}: touch targets at least 44 px on each side${r.small.length ? ` (${r.small.length}: ${r.small.slice(0, WORST).join('; ')})` : ''}`)) tally.touch++
        if (!check(r.defaults.length === 0, `${id}: no browser-default links (link color, underline outside prose) or controls (UA background and border)${r.defaults.length ? ` (${r.defaults.length}: ${r.defaults.slice(0, WORST).join('; ')})` : ''}`)) tally.defaults++
        const worst = r.contrast.sort((a, b) => a.ratio - b.ratio).slice(0, WORST).map((c) => `${c.what} ${c.ratio.toFixed(2)}<${c.need}`)
        if (!check(r.contrast.length === 0, `${id}: text contrast >= 4.5 (3 for large text) over the real background${r.gradient ? `, ${r.gradient} over gradients at their worst stop` : ''}${worst.length ? ` (${r.contrast.length}: ${worst.join('; ')})` : ''}`)) tally.contrast++
      }
    }
  }
  log('failing screens per rule', JSON.stringify(tally))
  log('caster', caster.username, 'live', liveId, 'ended', endedId, 'recording', recordingId)

  await castPage.bringToFront()
  await castPage.getByRole('button', { name: '방송 종료' }).click()
  check(await waitStatus(castPage, liveId, 'ENDED', 15_000), 'main broadcast turns ENDED after 방송 종료')
  liveId = undefined
  await phonePage.bringToFront()
  await phonePage.getByRole('button', { name: '방송 종료' }).click()
  check(await waitStatus(phonePage, phoneId, 'ENDED', 15_000), 'phone broadcast turns ENDED after 방송 종료')
  phoneId = undefined
  check((await api(castPage, 'DELETE', `/api/broadcasts/${readyId}`)).status === 204, 'the READY broadcast is deleted')
  await noCspViolations([['caster', castPage], ['viewer', viewerPage], ['phone-publisher', phonePage], ...pages])
} catch (e) {
  check(false, `journey aborted: ${e.message.split('\n')[0]}`)
} finally {
  if (castPage && liveId) await stopBroadcast(castPage, liveId)
  if (phonePage && phoneId) await stopBroadcast(phonePage, phoneId)
  await browser.close()
}
process.exit(summary() ? 0 : 1)
