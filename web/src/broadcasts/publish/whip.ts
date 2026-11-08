import { TARGET, type Capture } from './capture.ts'

export class WhipError extends Error {
  readonly status: number

  constructor(status: number) {
    super(`WHIP endpoint answered ${status}`)
    this.name = 'WhipError'
    this.status = status
  }
}

export type Publication = {
  peer: RTCPeerConnection
  close: () => Promise<void>
}

export type SendStats = { kbps: number; width?: number; height?: number; fps?: number }

function waitIceGathering(peer: RTCPeerConnection): Promise<void> {
  if (peer.iceGatheringState === 'complete') return Promise.resolve()
  return new Promise((resolve) => {
    peer.addEventListener('icegatheringstatechange', () => {
      if (peer.iceGatheringState === 'complete') resolve()
    })
    // Gathering can stall on hosts with unreachable interfaces; what was gathered is enough.
    setTimeout(resolve, 3000)
  })
}

function preferCodec(transceiver: RTCRtpTransceiver, mimeType: string) {
  const codecs = RTCRtpReceiver.getCapabilities('video')?.codecs ?? []
  const preferred = codecs.filter((c) => c.mimeType.toLowerCase() === mimeType.toLowerCase())
  if (preferred.length === 0) return
  transceiver.setCodecPreferences([...preferred, ...codecs.filter((c) => !preferred.includes(c))])
}

async function capSender(sender: RTCRtpSender, track: MediaStreamTrack) {
  const { height = TARGET.height } = track.getSettings()
  const params = sender.getParameters()
  const [encoding = {}] = params.encodings
  params.encodings = [
    {
      ...encoding,
      maxBitrate: TARGET.maxBitrate,
      maxFramerate: TARGET.fps,
      scaleResolutionDownBy: Math.max(1, height / TARGET.height),
    },
  ]
  await sender.setParameters(params)
}

// WHIP: POST the complete offer with the bearer token, get the answer and a session URL back.
export async function publish(capture: Capture, whipUrl: string, token: string): Promise<Publication> {
  const peer = new RTCPeerConnection()
  try {
    const video = peer.addTransceiver(capture.video, { direction: 'sendonly' })
    preferCodec(video, 'video/H264')
    if (capture.audio) peer.addTransceiver(capture.audio, { direction: 'sendonly' })

    await peer.setLocalDescription(await peer.createOffer())
    await waitIceGathering(peer)
    const response = await fetch(whipUrl, {
      method: 'POST',
      headers: { 'Content-Type': 'application/sdp', Authorization: `Bearer ${token}` },
      body: peer.localDescription?.sdp,
    })
    if (response.status !== 201) throw new WhipError(response.status)
    await peer.setRemoteDescription({ type: 'answer', sdp: await response.text() })
    await capSender(video.sender, capture.video)

    const session = response.headers.get('Location')
    return {
      peer,
      close: async () => {
        if (session) {
          await fetch(session, { method: 'DELETE', headers: { Authorization: `Bearer ${token}` } }).catch(() => {})
        }
        peer.close()
      },
    }
  } catch (error) {
    peer.close()
    throw error
  }
}

// How long a `disconnected` peer may take to come back before it counts as lost.
const RECONNECT_GRACE_MS = 5000

// Calls `onLost` once the media server is gone for good: it refused the publisher (someone else
// is publishing), kicked it, the token expired, or the network did not come back in time.
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

// Turns the cumulative video byte counter into a per-interval rate.
export function videoStatsSampler(peer: RTCPeerConnection): () => Promise<SendStats | null> {
  let last: { bytes: number; at: number } | null = null
  return async () => {
    const report = await peer.getStats()
    for (const entry of report.values()) {
      const stats = entry as RTCOutboundRtpStreamStats
      if (stats.type !== 'outbound-rtp' || stats.kind !== 'video') continue
      const bytes = stats.bytesSent ?? 0
      const kbps = last ? Math.round(((bytes - last.bytes) * 8) / (stats.timestamp - last.at)) : 0
      last = { bytes, at: stats.timestamp }
      return { kbps, width: stats.frameWidth, height: stats.frameHeight, fps: stats.framesPerSecond }
    }
    return null
  }
}
