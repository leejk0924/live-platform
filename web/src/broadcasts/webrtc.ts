export function waitIceGathering(peer: RTCPeerConnection): Promise<void> {
  if (peer.iceGatheringState === 'complete') return Promise.resolve()
  return new Promise((resolve) => {
    peer.addEventListener('icegatheringstatechange', () => {
      if (peer.iceGatheringState === 'complete') resolve()
    })
    // Gathering can stall on hosts with unreachable interfaces; what was gathered is enough.
    setTimeout(resolve, 3000)
  })
}

// How long a `disconnected` peer may take to come back before it counts as lost.
const RECONNECT_GRACE_MS = 5000

// Calls `onLost` once the media server is gone for good: it refused or kicked the peer (for a
// publisher: someone else is publishing, or the token expired), or the network did not come back in time.
// A peer closed locally fires no event, so `close()` never reports itself.
export function watchConnection(peer: RTCPeerConnection, onLost: () => void) {
  let grace: number | undefined
  const check = () => {
    window.clearTimeout(grace)
    if (peer.connectionState === 'failed') onLost()
    else if (peer.connectionState === 'disconnected') grace = window.setTimeout(onLost, RECONNECT_GRACE_MS)
  }
  peer.addEventListener('connectionstatechange', check)
  check()
}
