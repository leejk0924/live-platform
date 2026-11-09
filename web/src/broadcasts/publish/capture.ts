// Passthrough has no server-side transcoding, so the browser is the only place the
// 720p30 / 2.5 Mbps budget can hold.
export const TARGET = { width: 1280, height: 720, fps: 30, maxBitrate: 2_500_000 } as const

export type Source = 'screen' | 'camera'
export type Facing = 'user' | 'environment'

type CaptureBase = {
  video: MediaStreamTrack
  audio: MediaStreamTrack | null
  hasMic: boolean
  release: () => void
}

export type ScreenCapture = CaptureBase & { source: 'screen'; hasTabAudio: boolean }
export type CameraCapture = CaptureBase & { source: 'camera'; facing: Facing }
export type Capture = ScreenCapture | CameraCapture

type Devices = { getDisplayMedia?: unknown; getUserMedia?: unknown } | undefined

// Screen first, so a PC that can do both keeps screen sharing as the default.
export function availableSources(devices: Devices): Source[] {
  const sources: Source[] = []
  if (typeof devices?.getDisplayMedia === 'function') sources.push('screen')
  if (typeof devices?.getUserMedia === 'function') sources.push('camera')
  return sources
}

// Mobile browsers stop camera capture in the background while desktop browsers keep it, and a PC
// screen sharer switches tabs all the time. No getDisplayMedia is how a mobile browser shows itself.
export function pausesInBackground(source: Source, devices: Devices): boolean {
  return source === 'camera' && typeof devices?.getDisplayMedia !== 'function'
}

export function otherFacing(facing: Facing): Facing {
  return facing === 'user' ? 'environment' : 'user'
}

const CAMERA_SIZE = {
  width: { ideal: TARGET.width },
  height: { ideal: TARGET.height },
  frameRate: { ideal: TARGET.fps, max: TARGET.fps },
}

export function cameraVideoConstraints(facing: Facing): MediaTrackConstraints {
  return { facingMode: { exact: facing }, ...CAMERA_SIZE }
}

export function pickOtherCamera(
  devices: Pick<MediaDeviceInfo, 'kind' | 'deviceId'>[],
  currentDeviceId?: string,
): string | undefined {
  return devices.find((d) => d.kind === 'videoinput' && d.deviceId && d.deviceId !== currentDeviceId)?.deviceId
}

// The device asked for is not there: a desktop webcam reports no facing, or there is no microphone.
function isMissingDevice(error: unknown): boolean {
  return error instanceof DOMException && (error.name === 'NotFoundError' || error.name === 'OverconstrainedError')
}

async function cameraTrack(video: MediaTrackConstraints): Promise<MediaStreamTrack> {
  const stream = await navigator.mediaDevices.getUserMedia({ video })
  return stream.getVideoTracks()[0]
}

function cameraCapture(video: MediaStreamTrack, audio: MediaStreamTrack | null, facing: Facing): CameraCapture {
  video.contentHint = 'motion'
  return {
    source: 'camera',
    facing,
    video,
    audio,
    hasMic: audio !== null,
    release: () => {
      video.stop()
      audio?.stop()
    },
  }
}

// One getUserMedia call for camera and microphone, so iOS asks for permission once.
export async function captureCamera(facing: Facing): Promise<CameraCapture> {
  const audio = { echoCancellation: true }
  const loose = { ...CAMERA_SIZE, facingMode: facing }
  const attempts: MediaStreamConstraints[] = [
    { video: cameraVideoConstraints(facing), audio },
    { video: loose, audio },
    { video: loose },
  ]
  let failure: unknown
  for (const constraints of attempts) {
    try {
      const stream = await navigator.mediaDevices.getUserMedia(constraints)
      return cameraCapture(stream.getVideoTracks()[0], stream.getAudioTracks()[0] ?? null, facing)
    } catch (error) {
      if (!isMissingDevice(error)) throw error
      failure = error
    }
  }
  throw failure
}

// Carries the camera reopened on the old side, if that worked, so the broadcast keeps a picture.
export class CameraSwitchError extends Error {
  readonly fallback: CameraCapture | null

  constructor(cause: unknown, fallback: CameraCapture | null) {
    super('camera switch failed', { cause })
    this.name = 'CameraSwitchError'
    this.fallback = fallback
  }
}

// Stops the current camera before opening the other one: phones often allow only one open camera.
// The returned capture shares the microphone track with the one passed in.
export async function switchCamera(capture: CameraCapture): Promise<CameraCapture> {
  const { deviceId } = capture.video.getSettings()
  capture.video.stop()
  const target = otherFacing(capture.facing)
  try {
    const video = await cameraTrack(cameraVideoConstraints(target)).catch(async (error: unknown) => {
      if (!isMissingDevice(error)) throw error
      const other = pickOtherCamera(await navigator.mediaDevices.enumerateDevices(), deviceId)
      if (!other) throw error
      return cameraTrack({ ...CAMERA_SIZE, deviceId: { exact: other } })
    })
    const reported = video.getSettings().facingMode
    const facing = reported === 'user' || reported === 'environment' ? reported : target
    return cameraCapture(video, capture.audio, facing)
  } catch (error) {
    const video = await cameraTrack({ ...CAMERA_SIZE, facingMode: capture.facing }).catch(() => null)
    throw new CameraSwitchError(error, video && cameraCapture(video, capture.audio, capture.facing))
  }
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
export async function captureScreen(): Promise<ScreenCapture> {
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
    source: 'screen',
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
