import assert from 'node:assert/strict'
import { test, type TestContext } from 'node:test'
import { receive } from './whep.ts'

class FakePeer extends EventTarget {
  static last: FakePeer
  connectionState: RTCPeerConnectionState = 'new'
  iceGatheringState: RTCIceGatheringState = 'complete'
  localDescription: RTCSessionDescriptionInit | null = null
  closed = false
  // The state the peer moves to once it has the answer.
  static afterAnswer: RTCPeerConnectionState | null = 'connected'

  constructor() {
    super()
    FakePeer.last = this
  }

  addTransceiver() {}
  async createOffer() {
    return { type: 'offer', sdp: 'offer' } as const
  }
  async setLocalDescription(description: RTCSessionDescriptionInit) {
    this.localDescription = description
  }
  async setRemoteDescription() {
    const next = FakePeer.afterAnswer
    if (next) setTimeout(() => this.change(next))
  }
  close() {
    this.closed = true
    this.connectionState = 'closed'
  }
  change(state: RTCPeerConnectionState) {
    this.connectionState = state
    this.dispatchEvent(new Event('connectionstatechange'))
  }
}

type Call = { url: string; method: string; signal?: AbortSignal | null }

// Installs the fakes and returns the requests `receive` makes. `post` answers the WHEP POST.
function install(t: TestContext, post: (init: RequestInit) => Promise<Response>, afterAnswer: RTCPeerConnectionState | null = 'connected') {
  const calls: Call[] = []
  const saved = { fetch: globalThis.fetch, peer: globalThis.RTCPeerConnection, stream: globalThis.MediaStream }
  FakePeer.afterAnswer = afterAnswer
  globalThis.RTCPeerConnection = FakePeer as unknown as typeof RTCPeerConnection
  globalThis.MediaStream = class {
    addTrack() {}
  } as unknown as typeof MediaStream
  globalThis.fetch = (async (url: string, init: RequestInit = {}) => {
    const method = init.method ?? 'GET'
    calls.push({ url, method, signal: init.signal })
    return method === 'POST' ? post(init) : new Response(null, { status: 200 })
  }) as typeof fetch
  t.after(() => Object.assign(globalThis, { fetch: saved.fetch, RTCPeerConnection: saved.peer, MediaStream: saved.stream }))
  return calls
}

const created = async () => new Response('answer', { status: 201, headers: { Location: 'https://media/session/1' } })

test('receive works where AbortSignal.any is missing (Safari before 17.4)', async (t) => {
  const any = AbortSignal.any
  // @ts-expect-error simulating an older browser
  delete AbortSignal.any
  t.after(() => Object.assign(AbortSignal, { any }))
  const calls = install(t, created)
  const reception = await receive('https://media/whep', new AbortController().signal)
  assert.deepEqual(
    calls.map((c) => c.method),
    ['POST'],
  )
  reception.close()
})
