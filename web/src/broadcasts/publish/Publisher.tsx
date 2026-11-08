import { useEffect, useRef, useState } from 'react'
import { broadcastApi } from '../../api/broadcasts.ts'
import { errorMessage } from '../../api/client.ts'
import type { Broadcast } from '../../api/types.ts'
import { canCaptureScreen, captureScreen, type Capture } from './capture.ts'
import { publish, videoStatsSampler, watchConnection, WhipError, type Publication, type SendStats } from './whip.ts'

type Phase =
  | { kind: 'idle' }
  | { kind: 'capturing' }
  | { kind: 'connecting' }
  | { kind: 'publishing'; stats: SendStats | null }
  | { kind: 'stopping' }

type Session = { capture: Capture; publication: Publication | null; timer: number | undefined }

function audioNotice(capture: Capture): string | null {
  if (capture.hasTabAudio && capture.hasMic) return null
  if (!capture.hasTabAudio && !capture.hasMic) return '소리 없이 화면만 송출합니다.'
  if (!capture.hasTabAudio) return "탭/시스템 소리 없이 송출합니다. 공유 창에서 '오디오 공유'를 켜면 함께 보낼 수 있습니다."
  return '마이크를 쓸 수 없어 마이크 소리 없이 송출합니다.'
}

function failureMessage(error: unknown): string {
  if (error instanceof DOMException && error.name === 'NotAllowedError') return '화면 공유가 취소되었습니다.'
  if (error instanceof WhipError) return `송출 서버에 연결하지 못했습니다. (${error.status})`
  return errorMessage(error)
}

function formatStats(stats: SendStats): string {
  const size = stats.width && stats.height ? `${stats.width}x${stats.height}` : '-'
  const fps = stats.fps === undefined ? '-' : Math.round(stats.fps)
  return `${stats.kbps} kbps · ${size} · ${fps} fps`
}

export default function Publisher({ broadcast, onChanged }: { broadcast: Broadcast; onChanged: () => void }) {
  const [phase, setPhase] = useState<Phase>({ kind: 'idle' })
  const [notice, setNotice] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const session = useRef<Session | null>(null)
  const preview = useRef<HTMLVideoElement>(null)
  const supported = canCaptureScreen()

  function release() {
    const current = session.current
    session.current = null
    if (!current) return
    window.clearInterval(current.timer)
    void current.publication?.close()
    current.capture.release()
  }

  useEffect(() => release, [])

  async function stop() {
    setPhase({ kind: 'stopping' })
    setError(null)
    try {
      await broadcastApi.stop(broadcast.id)
    } catch (e) {
      setError(errorMessage(e))
    }
    release()
    setNotice(null)
    setPhase({ kind: 'idle' })
    onChanged()
  }

  async function start() {
    setError(null)
    setNotice(null)
    setPhase({ kind: 'capturing' })
    let capture: Capture
    try {
      capture = await captureScreen()
    } catch (e) {
      setError(failureMessage(e))
      setPhase({ kind: 'idle' })
      return
    }
    session.current = { capture, publication: null, timer: undefined }
    setNotice(audioNotice(capture))
    if (preview.current) preview.current.srcObject = new MediaStream([capture.video])
    // Ending the share from the browser's own bar ends the broadcast too.
    capture.video.addEventListener('ended', () => void stop())

    setPhase({ kind: 'connecting' })
    try {
      const grant = await broadcastApi.start(broadcast.id)
      const publication = await publish(capture, grant.whipUrl, grant.token)
      const current = session.current
      if (current?.capture !== capture) {
        // Stopped while connecting.
        void publication.close()
        return
      }
      const sample = videoStatsSampler(publication.peer)
      current.publication = publication
      current.timer = window.setInterval(async () => {
        const stats = await sample()
        setPhase((p) => (p.kind === 'publishing' ? { kind: 'publishing', stats } : p))
      }, 1000)
      setPhase({ kind: 'publishing', stats: null })
      // The broadcast stays LIVE: a dropped connection is not the owner ending the show.
      watchConnection(publication.peer, () => {
        if (session.current?.publication !== publication) return
        release()
        setNotice(null)
        setError('송출 서버와의 연결이 끊겼습니다. 다른 곳에서 이미 송출 중이거나 네트워크가 끊겼을 수 있습니다.')
        setPhase({ kind: 'idle' })
        onChanged()
      })
    } catch (e) {
      release()
      setError(failureMessage(e))
      setPhase({ kind: 'idle' })
    }
    onChanged()
  }

  const busy = phase.kind !== 'idle' && phase.kind !== 'publishing'
  const publishing = phase.kind === 'publishing'
  const canStart = supported && !publishing && (broadcast.status === 'READY' || broadcast.status === 'LIVE')
  const canStop = publishing || broadcast.status === 'LIVE'

  return (
    <section className="stack publisher" aria-label="송출">
      {!supported && <p className="error">이 브라우저는 화면 송출을 지원하지 않습니다. PC의 Chrome이나 Edge를 쓰세요.</p>}
      <div className="actions">
        {(canStart || !supported) && (
          <button type="button" onClick={start} disabled={busy || !supported}>
            방송 시작
          </button>
        )}
        {canStop && (
          <button type="button" onClick={stop} disabled={busy}>
            방송 종료
          </button>
        )}
      </div>
      <p className="publish-state" role="status">
        {phase.kind === 'capturing' && '공유할 화면을 고르세요…'}
        {phase.kind === 'connecting' && '송출 서버에 연결하는 중…'}
        {phase.kind === 'publishing' && `송출 중${phase.stats ? ` · ${formatStats(phase.stats)}` : ''}`}
        {phase.kind === 'stopping' && '방송을 종료하는 중…'}
      </p>
      {notice && <p className="notice">{notice}</p>}
      {error && (
        <p className="error" role="alert">
          {error}
        </p>
      )}
      <video ref={preview} className="preview" autoPlay muted playsInline hidden={phase.kind === 'idle'} />
    </section>
  )
}
