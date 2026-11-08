import { waitIceGathering } from '../webrtc.ts'

export type Reception = {
  peer: RTCPeerConnection
  stream: MediaStream
  close: () => void
}

// WHEP: POST the complete recvonly offer, get the answer and a session URL back.
// Watching needs no credentials, so unlike WHIP there is no bearer token.
export async function receive(whepUrl: string): Promise<Reception> {
  const peer = new RTCPeerConnection()
  const stream = new MediaStream()
  peer.addEventListener('track', (event) => stream.addTrack(event.track))
  try {
    peer.addTransceiver('video', { direction: 'recvonly' })
    peer.addTransceiver('audio', { direction: 'recvonly' })

    await peer.setLocalDescription(await peer.createOffer())
    await waitIceGathering(peer)
    const response = await fetch(whepUrl, {
      method: 'POST',
      headers: { 'Content-Type': 'application/sdp' },
      body: peer.localDescription?.sdp,
    })
    if (response.status !== 201) throw new Error(`WHEP endpoint answered ${response.status}`)
    await peer.setRemoteDescription({ type: 'answer', sdp: await response.text() })

    const session = response.headers.get('Location')
    return {
      peer,
      stream,
      close: () => {
        if (session) void fetch(session, { method: 'DELETE' }).catch(() => {})
        peer.close()
      },
    }
  } catch (error) {
    peer.close()
    throw error
  }
}
