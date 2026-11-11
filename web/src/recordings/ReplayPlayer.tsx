import { useEffect, useReducer, useRef } from 'react'
import { errorMessage } from '../api/client.ts'
import { recordingApi } from '../api/recordings.ts'
import { initialReplay, replayReducer } from './replay.ts'

// The replay URL expires within minutes, so a media error asks for a fresh one and picks up where it
// stopped. A fresh URL that fails again at once gives up (see replayReducer).
export default function ReplayPlayer({ recordingId }: { recordingId: string }) {
  const video = useRef<HTMLVideoElement>(null)
  const [state, dispatch] = useReducer(replayReducer, initialReplay)

  useEffect(() => {
    if (state.kind !== 'loading') return
    let active = true
    recordingApi.replay(recordingId).then(
      (replay) => active && dispatch({ type: 'urlLoaded', url: replay.url, now: Date.now() }),
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
  }

  function onError() {
    const element = video.current
    if (!element) return
    dispatch({ type: 'mediaError', at: { time: element.currentTime, playing: !element.paused }, now: Date.now() })
  }

  return (
    <section className="player" aria-label="다시보기" data-state={state.kind}>
      <div className="stage">
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
        {state.kind === 'loading' && <span className="spinner" aria-hidden="true" />}
        {state.kind === 'failed' && (
          <div className="stage-notice">
            <p className="error" role="alert">
              {state.error === null ? '영상을 재생하지 못했습니다.' : errorMessage(state.error)}
            </p>
            <button type="button" onClick={() => dispatch({ type: 'retry' })}>
              다시 시도
            </button>
          </div>
        )}
      </div>
      {state.kind === 'loading' && (
        <p className="player-status" role="status">
          영상을 불러오는 중…
        </p>
      )}
    </section>
  )
}
