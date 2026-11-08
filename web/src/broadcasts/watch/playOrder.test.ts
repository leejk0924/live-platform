import assert from 'node:assert/strict'
import { test } from 'node:test'
import type { PlaybackSource } from '../../api/types.ts'
import { isIos, playOrder } from './playOrder.ts'

const llHls: PlaybackSource = { kind: 'll-hls', url: '/media/hls/live/1/index.m3u8' }
const whep: PlaybackSource = { kind: 'whep', url: '/media/whep/live/1/whep' }
const serverOrder = [llHls, whep]

const orderCases = [
  { name: 'iOS gets WHEP only even when hls.js claims support', ios: true, hlsSupported: true, lowLatency: false, expected: ['whep'] },
  { name: 'iOS gets WHEP only with low latency on', ios: true, hlsSupported: true, lowLatency: true, expected: ['whep'] },
  { name: 'without hls.js support only WHEP is left', ios: false, hlsSupported: false, lowLatency: false, expected: ['whep'] },
  { name: 'desktop keeps the server order', ios: false, hlsSupported: true, lowLatency: false, expected: ['ll-hls', 'whep'] },
  { name: 'low latency puts WHEP first', ios: false, hlsSupported: true, lowLatency: true, expected: ['whep', 'll-hls'] },
]

for (const { name, expected, ...device } of orderCases) {
  test(`playOrder: ${name}`, () => {
    assert.deepEqual(
      playOrder(serverOrder, device).map((s) => s.kind),
      expected,
    )
  })
}

const iPhone = 'Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1'
const iPad = 'Mozilla/5.0 (iPad; CPU OS 17_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Mobile/15E148 Safari/604.1'
const mac = 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/605.1.15'
const windowsChrome = 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36'

const deviceCases = [
  { name: 'iPhone', userAgent: iPhone, platform: 'iPhone', maxTouchPoints: 5, expected: true },
  { name: 'iPad', userAgent: iPad, platform: 'iPad', maxTouchPoints: 5, expected: true },
  { name: 'iPadOS presenting as a Mac', userAgent: mac, platform: 'MacIntel', maxTouchPoints: 5, expected: true },
  { name: 'desktop Mac', userAgent: mac, platform: 'MacIntel', maxTouchPoints: 0, expected: false },
  { name: 'Windows Chrome', userAgent: windowsChrome, platform: 'Win32', maxTouchPoints: 0, expected: false },
]

for (const { name, expected, ...nav } of deviceCases) {
  test(`isIos: ${name} is ${expected}`, () => {
    assert.equal(isIos(nav as Navigator), expected)
  })
}
