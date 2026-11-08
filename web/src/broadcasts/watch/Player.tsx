import { useEffect, useRef, useState } from 'react'
import { broadcastApi } from '../../api/broadcasts.ts'
import type { PlaybackKind, PlaybackSource } from '../../api/types.ts'
import { watchConnection } from '../webrtc.ts'
import { isIos, playOrder } from './playOrder.ts'
import { receive } from './whep.ts'

type Playback = { kind: 'loading' } | { kind: 'playing'; source: PlaybackSource } | { kind: 'failed' }

// `lost` settles when the source can no longer play; `close` releases it.
type Attachment = { lost: Promise<void>; close: () => void }

const kindLabel: Record<PlaybackKind, string> = { 'll-hls': 'LL-HLS', whep: 'WebRTC' }

async function attachHls(video: HTMLVideoElement, url: string): Promise<Attachment> {
  const { default: Hls } = await import('hls.js')
  const manifest = Hls.DefaultConfig.manifestLoadPolicy.default
  const hls = new Hls({
    lowLatencyMode: true,
    // The media server answers 404 until the stream is ready, a moment after the broadcast turns
    // LIVE. hls.js gives up on a 404 at once, which would send every early viewer to WHEP.
    manifestLoadPolicy: {
      default: {
        ...manifest,
        errorRetry: {
          maxNumRetry: 5,
          retryDelayMs: 1000,
          maxRetryDelayMs: 2000,
          shouldRetry: (_config, count, _timeout, response, retry) => retry || (response?.code === 404 && count < 5),
        },
      },
    },
  })
  const lost = new Promise<void>((resolve) => {
    hls.on(Hls.Events.ERROR, (_event, data) => {
      if (data.fatal) resolve()
    })
  })
  hls.loadSource(url)
  hls.attachMedia(video)
  return { lost, close: () => hls.destroy() }
}

async function attachWhep(video: HTMLVideoElement, url: string): Promise<Attachment> {
  const reception = await receive(url)
  video.srcObject = reception.stream
  const lost = new Promise<void>((resolve) => watchConnection(reception.peer, resolve))
  return {
    lost,
    close: () => {
      reception.close()
      video.srcObject = null
    },
  }
}

const attachers: Record<PlaybackKind, (video: HTMLVideoElement, url: string) => Promise<Attachment>> = {
  'll-hls': attachHls,
  whep: attachWhep,
}

// Tries each source in order and falls through to the next when one fails. Once all have failed it
// calls `onLost`, so the page can check whether the broadcast is still on air. `onLost` must be stable.
export default function Player({ broadcastId, onLost }: { broadcastId: string; onLost: () => void }) {
  const video = useRef<HTMLVideoElement>(null)
  const [playback, setPlayback] = useState<Playback>({ kind: 'loading' })
  const [lowLatency, setLowLatency] = useState(false)
  const [attempt, setAttempt] = useState(0)
  const [muted, setMuted] = useState(true)
  const [ios] = useState(isIos)

  useEffect(() => {
    const element = video.current
    if (!element) return
    // One run per effect: cleanup marks it cancelled and releases whatever source it holds.
    const run: { cancelled: boolean; current: Attachment | null } = { cancelled: false, current: null }

    void (async () => {
      try {
        const sources = await broadcastApi.playback(broadcastId)
        const hlsSupported = !ios && (await import('hls.js')).default.isSupported()
        for (const source of playOrder(sources, { ios, hlsSupported, lowLatency })) {
          if (run.cancelled) return
          setPlayback({ kind: 'loading' })
          let attachment: Attachment
          try {
            attachment = await attachers[source.kind](element, source.url)
          } catch {
            continue
          }
          if (run.cancelled) return attachment.close()
          run.current = attachment
          setPlayback({ kind: 'playing', source })
          await attachment.lost
          if (run.cancelled) return
          attachment.close()
          run.current = null
        }
      } catch {
        // No sources: the broadcast is no longer on air, which the page learns through `onLost`.
      }
      if (run.cancelled) return
      setPlayback({ kind: 'failed' })
      onLost()
    })()

    return () => {
      run.cancelled = true
      run.current?.close()
    }
  }, [broadcastId, ios, lowLatency, attempt, onLost])

  function restart(change: () => void) {
    setPlayback({ kind: 'loading' })
    change()
  }

  function unmute() {
    if (video.current) video.current.muted = false
  }

  return (
    <section
      className="stack player"
      aria-label="시청"
      data-state={playback.kind}
      data-source={playback.kind === 'playing' ? playback.source.kind : undefined}
    >
      <video
        ref={video}
        autoPlay
        muted
        playsInline
        controls
        hidden={playback.kind === 'failed'}
        onVolumeChange={(e) => setMuted(e.currentTarget.muted)}
      />
      <div className="actions">
        {muted && playback.kind === 'playing' && (
          <button type="button" onClick={unmute}>
            소리 켜기
          </button>
        )}
        {!ios && (
          <label className="toggle">
            <input
              type="checkbox"
              checked={lowLatency}
              onChange={(e) => restart(() => setLowLatency(e.currentTarget.checked))}
            />
            저지연(WebRTC)
          </label>
        )}
      </div>
      <p role="status">
        {playback.kind === 'loading' && '영상을 불러오는 중…'}
        {playback.kind === 'playing' && `${kindLabel[playback.source.kind]}로 재생 중`}
      </p>
      {playback.kind === 'failed' && (
        <div className="stack">
          <p className="error" role="alert">
            영상을 재생하지 못했습니다.
          </p>
          <div className="actions">
            <button type="button" onClick={() => restart(() => setAttempt((n) => n + 1))}>
              다시 시도
            </button>
          </div>
        </div>
      )}
    </section>
  )
}
