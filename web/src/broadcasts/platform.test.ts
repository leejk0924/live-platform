import assert from 'node:assert/strict'
import { test } from 'node:test'
import { isIos } from './platform.ts'

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
    assert.equal(isIos(nav), expected)
  })
}
