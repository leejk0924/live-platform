export function waitIceGathering(peer: RTCPeerConnection): Promise<void> {
  if (peer.iceGatheringState === 'complete') return Promise.resolve()
  return new Promise((resolve) => {
    const done = () => {
      clearTimeout(timer)
      peer.removeEventListener('icegatheringstatechange', check)
      resolve()
    }
    const check = () => {
      if (peer.iceGatheringState === 'complete') done()
    }
    peer.addEventListener('icegatheringstatechange', check)
    // Gathering can stall on hosts with unreachable interfaces; what was gathered is enough.
    const timer = setTimeout(done, 3000)
  })
}

// How long a `disconnected` peer may take to come back before it counts as lost.
const RECONNECT_GRACE_MS = 5000

// Calls `onLost` once the media server is gone for good: it refused or kicked the peer (for a
// publisher: someone else is publishing, or the token expired), or the network did not come back in time.
// A peer closed locally fires no event, so `close()` never reports itself.
// Returns an unsubscribe, which also cancels a pending grace period.
export function watchConnection(peer: RTCPeerConnection, onLost: () => void): () => void {
  let grace: ReturnType<typeof setTimeout> | undefined
  const check = () => {
    clearTimeout(grace)
    if (peer.connectionState === 'failed') onLost()
    else if (peer.connectionState === 'disconnected') grace = setTimeout(onLost, RECONNECT_GRACE_MS)
  }
  peer.addEventListener('connectionstatechange', check)
  check()
  return () => {
    clearTimeout(grace)
    peer.removeEventListener('connectionstatechange', check)
  }
}
