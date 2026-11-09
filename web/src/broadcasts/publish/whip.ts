import { waitIceGathering } from '../webrtc.ts'
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
  // Swaps the sent picture in place: the WHIP session and its negotiation stay as they are.
  replaceVideo: (track: MediaStreamTrack) => Promise<void>
  close: () => Promise<void>
}

export type SendStats = { kbps: number; width?: number; height?: number; fps?: number }

function preferCodec(transceiver: RTCRtpTransceiver, mimeType: string) {
  const codecs = RTCRtpReceiver.getCapabilities('video')?.codecs ?? []
  const preferred = codecs.filter((c) => c.mimeType.toLowerCase() === mimeType.toLowerCase())
  if (preferred.length === 0) return
  transceiver.setCodecPreferences([...preferred, ...codecs.filter((c) => !preferred.includes(c))])
}

// The 720 budget applies to the shorter side, so a portrait 720x1280 phone frame is not shrunk to 405x720.
export function scaleDownBy({ width, height }: { width?: number; height?: number }): number {
  const sides = [width, height].filter((side): side is number => side !== undefined && side > 0)
  if (sides.length === 0) return 1
  return Math.max(1, Math.min(...sides) / TARGET.height)
}

async function capSender(sender: VideoSender, track: MediaStreamTrack) {
  const params = sender.getParameters()
  const [encoding = {}] = params.encodings
  params.encodings = [
    {
      ...encoding,
      maxBitrate: TARGET.maxBitrate,
      maxFramerate: TARGET.fps,
      scaleResolutionDownBy: scaleDownBy(track.getSettings()),
    },
  ]
  await sender.setParameters(params)
}

type VideoSender = Pick<RTCRtpSender, 'replaceTrack' | 'getParameters' | 'setParameters'>

// Fails only when the sender keeps the old track. A missed bitrate cap still sends the new picture.
export async function replaceSenderVideo(sender: VideoSender, track: MediaStreamTrack): Promise<void> {
  await sender.replaceTrack(track)
  await capSender(sender, track).catch(() => {})
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
      replaceVideo: (track) => replaceSenderVideo(video.sender, track),
      close: async () => {
        if (session) {
          // keepalive lets the DELETE outlive a page that is being hidden or unloaded.
          await fetch(session, {
            method: 'DELETE',
            headers: { Authorization: `Bearer ${token}` },
            keepalive: true,
          }).catch(() => {})
        }
        peer.close()
      },
    }
  } catch (error) {
    peer.close()
    throw error
  }
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
