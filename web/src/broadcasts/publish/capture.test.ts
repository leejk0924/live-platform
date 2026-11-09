import assert from 'node:assert/strict'
import { test } from 'node:test'
import {
  cameraVideoConstraints,
  otherFacing,
  pickOtherCamera,
  publishOptions,
  switchSentCamera,
  type CameraCapture,
  type Facing,
} from './capture.ts'

const iPhone = 'Mozilla/5.0 (iPhone; CPU iPhone OS 27_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/27.0 Mobile/15E148 Safari/604.1'
const android = 'Mozilla/5.0 (Linux; Android 15; Pixel 9) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Mobile Safari/537.36'
const mac = 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/27.0 Safari/605.1.15'
const windowsChrome = 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36'

const screenAndCamera = { getDisplayMedia: () => {}, getUserMedia: () => {} }
const cameraOnly = { getUserMedia: () => {} }

const platformCases = [
  {
    name: 'an iPhone whose Safari can share the screen',
    nav: { userAgent: iPhone, platform: 'iPhone', maxTouchPoints: 5, mediaDevices: screenAndCamera },
    expected: { sources: ['camera'], pausesInBackground: true },
  },
  {
    name: 'an Android phone',
    nav: { userAgent: android, platform: 'Linux armv81', maxTouchPoints: 5, mediaDevices: cameraOnly },
    expected: { sources: ['camera'], pausesInBackground: true },
  },
  {
    name: 'an iPad presenting as a Mac',
    nav: { userAgent: mac, platform: 'MacIntel', maxTouchPoints: 5, mediaDevices: screenAndCamera },
    expected: { sources: ['camera'], pausesInBackground: true },
  },
  {
    name: 'a Chromium phone that says so without Android in its user agent',
    nav: { userAgent: windowsChrome, platform: 'Win32', maxTouchPoints: 5, userAgentData: { mobile: true }, mediaDevices: cameraOnly },
    expected: { sources: ['camera'], pausesInBackground: true },
  },
  {
    name: 'desktop Chrome',
    nav: { userAgent: windowsChrome, platform: 'Win32', maxTouchPoints: 0, userAgentData: { mobile: false }, mediaDevices: screenAndCamera },
    expected: { sources: ['screen', 'camera'], pausesInBackground: false },
  },
  {
    name: 'a desktop Mac',
    nav: { userAgent: mac, platform: 'MacIntel', maxTouchPoints: 0, mediaDevices: screenAndCamera },
    expected: { sources: ['screen', 'camera'], pausesInBackground: false },
  },
  {
    name: 'an insecure desktop page with no media devices',
    nav: { userAgent: windowsChrome, platform: 'Win32', maxTouchPoints: 0, mediaDevices: undefined },
    expected: { sources: [], pausesInBackground: false },
  },
]

for (const { name, nav, expected } of platformCases) {
  test(`publishOptions: ${name}`, () => {
    assert.deepEqual(publishOptions(nav), expected)
  })
}

test('otherFacing flips between front and back', () => {
  assert.equal(otherFacing('user'), 'environment')
  assert.equal(otherFacing('environment'), 'user')
})

test('camera constraints demand the facing and ask for 720p30', () => {
  assert.deepEqual(cameraVideoConstraints('environment'), {
    facingMode: { exact: 'environment' },
    width: { ideal: 1280 },
    height: { ideal: 720 },
    frameRate: { ideal: 30, max: 30 },
  })
})

test('pickOtherCamera picks the first other video input', () => {
  const devices = [
    { kind: 'audioinput', deviceId: 'mic' },
    { kind: 'videoinput', deviceId: 'front' },
    { kind: 'videoinput', deviceId: 'back' },
    { kind: 'videoinput', deviceId: 'wide' },
  ] as const
  assert.equal(pickOtherCamera([...devices], 'front'), 'back')
  assert.equal(pickOtherCamera([...devices]), 'front')
})

test('pickOtherCamera finds nothing with a single camera', () => {
  const devices = [{ kind: 'videoinput', deviceId: 'front' }] as const
  assert.equal(pickOtherCamera([...devices], 'front'), undefined)
})

type FakeTrack = MediaStreamTrack & { stopped: boolean }

function fakeTrack(facingMode: Facing, deviceId: string = facingMode): FakeTrack {
  const track = {
    stopped: false,
    contentHint: '',
    getSettings: () => ({ facingMode, deviceId }),
    stop: () => {
      track.stopped = true
    },
  }
  return track as unknown as FakeTrack
}

function frontCamera(): CameraCapture {
  const video = fakeTrack('user')
  return { source: 'camera', facing: 'user', video, audio: null, hasMic: false, release: () => video.stop() }
}

async function withCameras(answers: (() => MediaStreamTrack)[], run: () => Promise<void>) {
  const original = Object.getOwnPropertyDescriptor(globalThis, 'navigator')
  const getUserMedia = async () => {
    const answer = answers.shift()
    if (!answer) throw new Error('unexpected getUserMedia call')
    const track = answer()
    return { getVideoTracks: () => [track] }
  }
  Object.defineProperty(globalThis, 'navigator', {
    value: { mediaDevices: { getUserMedia, enumerateDevices: async () => [] } },
    configurable: true,
  })
  try {
    await run()
  } finally {
    if (original) Object.defineProperty(globalThis, 'navigator', original)
  }
}

const busyCamera = () => {
  throw new DOMException('camera in use', 'NotReadableError')
}

test('a camera switch sends the other camera', async () => {
  const back = fakeTrack('environment')
  await withCameras([() => back], async () => {
    const sent: MediaStreamTrack[] = []
    const outcome = await switchSentCamera(frontCamera(), async (track) => {
      sent.push(track)
    })
    assert.equal(outcome.kind, 'switched')
    assert.equal(outcome.kind === 'switched' && outcome.capture.facing, 'environment')
    assert.deepEqual(sent, [back])
  })
})

test('a camera the sender cannot take is lost and closed', async () => {
  const back = fakeTrack('environment')
  await withCameras([() => back], async () => {
    const outcome = await switchSentCamera(frontCamera(), async () => {
      throw new Error('replaceTrack failed')
    })
    assert.deepEqual(outcome, { kind: 'lost' })
    assert.equal(back.stopped, true, 'the camera nobody sends is closed')
  })
})

test('a failed switch keeps sending the camera reopened on the old side', async () => {
  const reopened = fakeTrack('user', 'front-again')
  await withCameras([busyCamera, () => reopened], async () => {
    const sent: MediaStreamTrack[] = []
    const outcome = await switchSentCamera(frontCamera(), async (track) => {
      sent.push(track)
    })
    assert.equal(outcome.kind, 'kept')
    assert.equal(outcome.kind === 'kept' && outcome.capture.facing, 'user')
    assert.deepEqual(sent, [reopened])
  })
})

test('a switch that cannot reopen any camera is lost', async () => {
  await withCameras([busyCamera, busyCamera], async () => {
    const outcome = await switchSentCamera(frontCamera(), async () => {})
    assert.deepEqual(outcome, { kind: 'lost' })
  })
})
