// Passthrough has no server-side transcoding, so the browser is the only place the
// 720p30 / 2.5 Mbps budget can hold.
export const TARGET = { height: 720, fps: 30, maxBitrate: 2_500_000 } as const

export type Capture = {
  video: MediaStreamTrack
  audio: MediaStreamTrack | null
  hasTabAudio: boolean
  hasMic: boolean
  release: () => void
}

export function canCaptureScreen(): boolean {
  return typeof navigator.mediaDevices?.getDisplayMedia === 'function'
}

// WHIP carries one audio track, so tab audio and the microphone are summed in Web Audio.
function mixAudio(tracks: MediaStreamTrack[]): { track: MediaStreamTrack | null; close: () => void } {
  if (tracks.length < 2) return { track: tracks[0] ?? null, close: () => {} }
  const context = new AudioContext()
  const destination = context.createMediaStreamDestination()
  for (const track of tracks) context.createMediaStreamSource(new MediaStream([track])).connect(destination)
  return { track: destination.stream.getAudioTracks()[0] ?? null, close: () => void context.close() }
}

// Asks for the screen first: browsers allow it only right after a click.
export async function captureScreen(): Promise<Capture> {
  const screen = await navigator.mediaDevices.getDisplayMedia({
    video: { height: { ideal: TARGET.height }, frameRate: { ideal: TARGET.fps, max: TARGET.fps } },
    audio: true,
  })
  // A denied or missing microphone still leaves a broadcast worth sending.
  const mic = await navigator.mediaDevices.getUserMedia({ audio: { echoCancellation: true } }).catch(() => null)
  const tabAudio = screen.getAudioTracks()
  const micAudio = mic?.getAudioTracks() ?? []
  const mixed = mixAudio([...tabAudio, ...micAudio])
  const [video] = screen.getVideoTracks()
  video.contentHint = 'detail'
  const owned = [...screen.getTracks(), ...(mic?.getTracks() ?? [])]
  return {
    video,
    audio: mixed.track,
    hasTabAudio: tabAudio.length > 0,
    hasMic: micAudio.length > 0,
    release: () => {
      for (const track of owned) track.stop()
      mixed.close()
    },
  }
}
