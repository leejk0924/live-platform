// Shared by publish.html and play.html. Throwaway spike code for issue #3; the real client lands with the broadcast feature.

export const TARGET = { height: 720, fps: 30, maxBitrate: 2_500_000 }

export function mediaUrl(kind, path) {
  return `/media/${kind}/${encodeURIComponent(path)}`
}

function waitIceGathering(pc) {
  if (pc.iceGatheringState === 'complete') return Promise.resolve()
  return new Promise((resolve) => {
    const done = () => {
      if (pc.iceGatheringState === 'complete') resolve()
    }
    pc.addEventListener('icegatheringstatechange', done)
    // Gathering can stall on hosts with unreachable interfaces; what has been gathered is enough on a LAN.
    setTimeout(resolve, 3000)
  })
}

// WHIP and WHEP share one exchange: POST the full offer, get the answer and a session URL back.
export async function negotiate(pc, endpoint, headers = {}) {
  await pc.setLocalDescription(await pc.createOffer())
  await waitIceGathering(pc)
  const res = await fetch(endpoint, {
    method: 'POST',
    headers: { 'Content-Type': 'application/sdp', ...headers },
    body: pc.localDescription.sdp,
  })
  if (res.status !== 201) throw new Error(`${endpoint} answered ${res.status}: ${await res.text()}`)
  await pc.setRemoteDescription({ type: 'answer', sdp: await res.text() })
  return res.headers.get('Location')
}

export function preferCodec(transceiver, mimeType) {
  const codecs = RTCRtpReceiver.getCapabilities(transceiver.receiver.track.kind)?.codecs ?? []
  const preferred = codecs.filter((c) => c.mimeType.toLowerCase() === mimeType.toLowerCase())
  if (preferred.length === 0) return false
  transceiver.setCodecPreferences([...preferred, ...codecs.filter((c) => !preferred.includes(c))])
  return true
}

// Turns cumulative getStats counters into per-second rates for one RTP direction ('outbound-rtp' or 'inbound-rtp').
export function rtpSampler(pc, type) {
  let last = {}
  return async () => {
    const out = {}
    for (const s of (await pc.getStats()).values()) {
      if (s.type !== type) continue
      const bytes = s.bytesSent ?? s.bytesReceived ?? 0
      const prev = last[s.kind]
      const kbps = prev ? Math.round(((bytes - prev.bytes) * 8) / (s.timestamp - prev.ts)) : 0
      last[s.kind] = { bytes, ts: s.timestamp }
      out[s.kind] = {
        kbps,
        width: s.frameWidth,
        height: s.frameHeight,
        fps: s.framesPerSecond,
        limitedBy: s.qualityLimitationReason,
        packetsLost: s.packetsLost,
      }
    }
    return out
  }
}
