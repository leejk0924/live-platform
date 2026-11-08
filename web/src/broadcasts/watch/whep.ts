import { waitConnected, waitIceGathering } from '../webrtc.ts'

export type Reception = {
  peer: RTCPeerConnection
  stream: MediaStream
  close: () => void
}

// Covers the answer and the connection both: a media server that never answers, or a peer that
// never connects, would otherwise hold the player instead of letting it fall through to the next source.
const CONNECT_TIMEOUT_MS = 10_000

// WHEP: POST the complete recvonly offer, get the answer and a session URL back.
// Watching needs no credentials, so unlike WHIP there is no bearer token.
// Resolves only once media can flow, so a caller never shows a peer that is still connecting.
export async function receive(whepUrl: string, signal: AbortSignal): Promise<Reception> {
  // Hand-rolled rather than AbortSignal.any, which iOS lacks before 17.4, and iOS plays only through WHEP.
  const deadline = new AbortController()
  const abort = () => deadline.abort(signal.reason)
  if (signal.aborted) abort()
  else signal.addEventListener('abort', abort, { once: true })
  const timer = setTimeout(() => deadline.abort(new DOMException('WHEP did not connect in time', 'TimeoutError')), CONNECT_TIMEOUT_MS)
  const peer = new RTCPeerConnection()
  const stream = new MediaStream()
  peer.addEventListener('track', (event) => stream.addTrack(event.track))
  let session: string | null = null
  const close = () => {
    if (session) void fetch(session, { method: 'DELETE' }).catch(() => {})
    peer.close()
  }
  try {
    peer.addTransceiver('video', { direction: 'recvonly' })
    peer.addTransceiver('audio', { direction: 'recvonly' })

    await peer.setLocalDescription(await peer.createOffer())
    await waitIceGathering(peer)
    // Aborted after the server created the session but before the 201 arrived, there is no Location
    // to DELETE; the media server drops that session itself when its WebRTC handshake times out.
    const response = await fetch(whepUrl, {
      method: 'POST',
      headers: { 'Content-Type': 'application/sdp' },
      body: peer.localDescription?.sdp,
      signal: deadline.signal,
    })
    if (response.status !== 201) throw new Error(`WHEP endpoint answered ${response.status}`)
    session = response.headers.get('Location')
    await peer.setRemoteDescription({ type: 'answer', sdp: await response.text() })
    await waitConnected(peer, deadline.signal)
    return { peer, stream, close }
  } catch (error) {
    close()
    throw error
  } finally {
    clearTimeout(timer)
    signal.removeEventListener('abort', abort)
  }
}
