import assert from 'node:assert/strict'
import { test } from 'node:test'
import type { PlaybackSource } from '../../api/types.ts'
import { playOrder } from './playOrder.ts'

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
