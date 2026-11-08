import { useEffect, useReducer, useRef } from 'react'
import { errorMessage } from '../api/client.ts'
import { recordingApi } from '../api/recordings.ts'
import { initialReplay, replayReducer } from './replay.ts'

// The replay URL expires, so a media error asks for a fresh one once and picks up where it stopped.
export default function ReplayPlayer({ recordingId }: { recordingId: string }) {
  const video = useRef<HTMLVideoElement>(null)
  const [state, dispatch] = useReducer(replayReducer, initialReplay)

  useEffect(() => {
    if (state.kind !== 'loading') return
    let active = true
    recordingApi.replay(recordingId).then(
      (replay) => active && dispatch({ type: 'urlLoaded', url: replay.url }),
      (error: unknown) => active && dispatch({ type: 'urlFailed', error }),
    )
    return () => {
      active = false
    }
  }, [state.kind, recordingId])

  function onLoadedMetadata() {
    const element = video.current
    if (!element || state.kind !== 'ready') return
    if (state.resume) {
      element.currentTime = state.resume.time
      if (state.resume.playing) element.play().catch(() => {})
    }
    dispatch({ type: 'mediaLoaded' })
  }

  function onError() {
    const element = video.current
    if (!element) return
    dispatch({ type: 'mediaError', at: { time: element.currentTime, playing: !element.paused } })
  }

  return (
    <section className="stack player" aria-label="다시보기" data-state={state.kind}>
      <video
        ref={video}
        controls
        playsInline
        preload="metadata"
        src={state.kind === 'ready' ? state.url : undefined}
        hidden={state.kind === 'failed'}
        onLoadedMetadata={onLoadedMetadata}
        onError={onError}
      />
      {state.kind === 'loading' && <p role="status">영상을 불러오는 중…</p>}
      {state.kind === 'failed' && (
        <div className="stack">
          <p className="error" role="alert">
            {state.error === null ? '영상을 재생하지 못했습니다.' : errorMessage(state.error)}
          </p>
          <div className="actions">
            <button type="button" onClick={() => dispatch({ type: 'retry' })}>
              다시 시도
            </button>
          </div>
        </div>
      )}
    </section>
  )
}
