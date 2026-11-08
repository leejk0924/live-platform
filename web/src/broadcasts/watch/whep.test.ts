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

function signalled() {
  let resolve = () => {}
  const promise = new Promise<void>((r) => (resolve = r))
  return { promise, resolve }
}

// Answers like a real fetch that never hears back: pending until its signal aborts.
function hang(posted: () => void) {
  return (init: RequestInit) =>
    new Promise<Response>((_, reject) => {
      posted()
      init.signal?.addEventListener('abort', () => reject(init.signal?.reason))
    })
}

test('a WHEP POST that never answers gives up at the deadline and closes the peer', async (t) => {
  t.mock.timers.enable({ apis: ['setTimeout'] })
  const { promise: posted, resolve } = signalled()
  const calls = install(t, hang(resolve))
  const reception = receive('https://media/whep', new AbortController().signal)
  await posted
  t.mock.timers.tick(10_000)
  await assert.rejects(reception, { name: 'TimeoutError' })
  assert.equal(calls[0].signal?.aborted, true)
  assert.equal(FakePeer.last.closed, true)
  assert.deepEqual(
    calls.map((c) => c.method),
    ['POST'],
  )
})

test('aborting the caller signal cancels the WHEP POST', async (t) => {
  const { promise: posted, resolve } = signalled()
  const calls = install(t, hang(resolve))
  const caller = new AbortController()
  const reception = receive('https://media/whep', caller.signal)
  await posted
  caller.abort(new Error('player left'))
  await assert.rejects(reception, /player left/)
  assert.equal(calls[0].signal?.aborted, true)
  assert.equal(FakePeer.last.closed, true)
})

test('a peer that fails after the answer deletes the WHEP session once', async (t) => {
  const calls = install(t, created, 'failed')
  await assert.rejects(receive('https://media/whep', new AbortController().signal), /failed/)
  assert.deepEqual(
    calls.map((c) => [c.method, c.url]),
    [
      ['POST', 'https://media/whep'],
      ['DELETE', 'https://media/session/1'],
    ],
  )
  assert.equal(FakePeer.last.closed, true)
})

test('a connected peer keeps its session until close', async (t) => {
  const calls = install(t, created)
  const reception = await receive('https://media/whep', new AbortController().signal)
  assert.equal(reception.peer.connectionState, 'connected')
  assert.equal(calls.length, 1)
  reception.close()
  assert.deepEqual(
    calls.map((c) => [c.method, c.url]),
    [
      ['POST', 'https://media/whep'],
      ['DELETE', 'https://media/session/1'],
    ],
  )
  assert.equal(FakePeer.last.closed, true)
})
