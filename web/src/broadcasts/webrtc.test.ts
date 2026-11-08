import assert from 'node:assert/strict'
import { mock, test } from 'node:test'
import { waitIceGathering, watchConnection } from './webrtc.ts'

class FakePeer extends EventTarget {
  connectionState: RTCPeerConnectionState = 'connected'
  iceGatheringState: RTCIceGatheringState = 'gathering'
  listeners = 0

  override addEventListener(...args: Parameters<EventTarget['addEventListener']>) {
    this.listeners++
    super.addEventListener(...args)
  }

  override removeEventListener(...args: Parameters<EventTarget['removeEventListener']>) {
    this.listeners--
    super.removeEventListener(...args)
  }

  change(state: RTCPeerConnectionState) {
    this.connectionState = state
    this.dispatchEvent(new Event('connectionstatechange'))
  }
}

const asPeer = (peer: FakePeer) => peer as unknown as RTCPeerConnection

test('watchConnection reports a failed peer', () => {
  const peer = new FakePeer()
  const onLost = mock.fn()
  watchConnection(asPeer(peer), onLost)
  peer.change('failed')
  assert.equal(onLost.mock.callCount(), 1)
})

test('watchConnection stays quiet after unsubscribing', () => {
  const peer = new FakePeer()
  const onLost = mock.fn()
  const unsubscribe = watchConnection(asPeer(peer), onLost)
  unsubscribe()
  peer.change('failed')
  assert.equal(onLost.mock.callCount(), 0)
  assert.equal(peer.listeners, 0)
})

test('unsubscribing cancels a pending reconnect grace', (t) => {
  t.mock.timers.enable({ apis: ['setTimeout'] })
  const peer = new FakePeer()
  const onLost = mock.fn()
  const unsubscribe = watchConnection(asPeer(peer), onLost)
  peer.change('disconnected')
  unsubscribe()
  t.mock.timers.tick(60_000)
  assert.equal(onLost.mock.callCount(), 0)
})

test('waitIceGathering lets go of the peer once gathering completes', async () => {
  const peer = new FakePeer()
  const done = waitIceGathering(asPeer(peer))
  peer.iceGatheringState = 'complete'
  peer.dispatchEvent(new Event('icegatheringstatechange'))
  await done
  assert.equal(peer.listeners, 0)
})
