import { useEffect, useRef, useState } from 'react'
import { broadcastApi } from '../../api/broadcasts.ts'
import { errorMessage } from '../../api/client.ts'
import type { Broadcast } from '../../api/types.ts'
import {
  captureCamera,
  captureScreen,
  otherFacing,
  publishOptions,
  switchSentCamera,
  type Capture,
  type Facing,
  type Source,
} from './capture.ts'
import { watchConnection } from '../webrtc.ts'
import { backgroundAction, type Phase } from './phase.ts'
import { publish, videoStatsSampler, WhipError, type Publication, type SendStats } from './whip.ts'

type Session = {
  capture: Capture
  publication: Publication | null
  timer: number | undefined
  unwatch: (() => void) | undefined
  wakeLock: WakeLockSentinel | null
}

const sourceLabel: Record<Source, string> = { screen: '화면', camera: '카메라' }
const facingLabel: Record<Facing, string> = { user: '전면 카메라', environment: '후면 카메라' }

const CONNECTION_LOST =
  '송출 서버와의 연결이 끊겼습니다. 다른 곳에서 이미 송출 중이거나 네트워크가 끊겼을 수 있습니다. 30초 안에 다시 시작하면 방송이 이어집니다.'
const CAMERA_LOST = '카메라가 꺼졌습니다. 다른 앱이 카메라를 쓰고 있을 수 있습니다. 30초 안에 다시 시작하면 방송이 이어집니다.'

function audioNotice(capture: Capture): string | null {
  if (capture.source === 'camera') return capture.hasMic ? null : '마이크 없이 영상만 송출합니다.'
  if (capture.hasTabAudio && capture.hasMic) return null
  if (!capture.hasTabAudio && !capture.hasMic) return '소리 없이 화면만 송출합니다.'
  if (!capture.hasTabAudio) return "탭/시스템 소리 없이 송출합니다. 공유 창에서 '오디오 공유'를 켜면 함께 보낼 수 있습니다."
  return '마이크를 쓸 수 없어 마이크 소리 없이 송출합니다.'
}

function failureMessage(error: unknown, source: Source): string {
  if (error instanceof DOMException && source === 'camera') {
    if (error.name === 'NotAllowedError') return '카메라·마이크 권한이 거부되었습니다. 브라우저 설정에서 허용한 뒤 다시 시작하세요.'
    if (error.name === 'NotReadableError') return '카메라를 다른 앱이 쓰고 있습니다.'
    if (error.name === 'NotFoundError' || error.name === 'OverconstrainedError') return '카메라를 찾지 못했습니다.'
  }
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
  const [{ sources, pausesInBackground: pauses }] = useState(() => publishOptions())
  const [source, setSource] = useState<Source>(sources[0] ?? 'screen')
  const [facing, setFacing] = useState<Facing>('user')
  const [switching, setSwitching] = useState(false)
  const [phase, setPhase] = useState<Phase>({ kind: 'idle' })
  const [notice, setNotice] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const session = useRef<Session | null>(null)
  const preview = useRef<HTMLVideoElement>(null)
  const starting = useRef(false)
  const supported = sources.length > 0

  function release() {
    const current = session.current
    session.current = null
    if (!current) return
    window.clearInterval(current.timer)
    current.unwatch?.()
    void current.publication?.close()
    current.capture.release()
    void current.wakeLock?.release().catch(() => {})
  }

  useEffect(() => release, [])

  // A reconnect makes the broadcast LIVE again only once the media server reports the stream,
  // a moment after the connection opens, so look again until that has happened.
  const resuming = phase.kind === 'publishing' && broadcast.status === 'INTERRUPTED'
  useEffect(() => {
    if (!resuming) return
    const timer = window.setTimeout(onChanged, 1000)
    return () => window.clearTimeout(timer)
  }, [resuming, broadcast, onChanged])

  // The broadcast stays LIVE: losing the connection or the camera is not the owner ending the show.
  function lose(message: string) {
    release()
    setNotice(null)
    setError(message)
    setPhase({ kind: 'idle' })
    onChanged()
  }

  function pause() {
    release()
    setNotice(null)
    setPhase({ kind: 'paused', since: Date.now() })
    onChanged()
  }

  function show(capture: Capture) {
    if (preview.current) preview.current.srcObject = new MediaStream([capture.video])
    const video = capture.video
    video.addEventListener(
      'ended',
      () => {
        // A track replaced by a camera switch belongs to no session any more.
        if (session.current?.capture.video !== video) return
        // Ending the share from the browser's own bar ends the broadcast too.
        if (capture.source === 'screen') void stop()
        else lose(CAMERA_LOST)
      },
      { once: true },
    )
  }

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

  // Two quick returns to the page can both see the paused phase before it re-renders.
  async function start() {
    if (starting.current) return
    starting.current = true
    try {
      await begin()
    } finally {
      starting.current = false
    }
  }

  async function begin() {
    setError(null)
    setNotice(null)
    setPhase({ kind: 'capturing' })
    let capture: Capture
    try {
      capture = source === 'camera' ? await captureCamera(facing) : await captureScreen()
    } catch (e) {
      setError(failureMessage(e, source))
      setPhase({ kind: 'idle' })
      return
    }
    session.current = { capture, publication: null, timer: undefined, unwatch: undefined, wakeLock: null }
    setNotice(audioNotice(capture))
    show(capture)

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
      current.unwatch = watchConnection(publication.peer, () => {
        if (session.current?.publication === publication) lose(CONNECTION_LOST)
      })
      // Keeps a phone screen from sleeping mid-broadcast; without it the broadcast still works.
      navigator.wakeLock
        ?.request('screen')
        .then((lock) => {
          if (session.current === current) current.wakeLock = lock
          else void lock.release()
        })
        .catch(() => {})
    } catch (e) {
      release()
      setError(failureMessage(e, source))
      setPhase({ kind: 'idle' })
    }
    onChanged()
  }

  async function flipCamera() {
    const current = session.current
    const publication = current?.publication
    if (!current || !publication || current.capture.source !== 'camera') {
      setFacing(otherFacing(facing))
      return
    }
    setSwitching(true)
    setError(null)
    const outcome = await switchSentCamera(current.capture, publication.replaceVideo)
    setSwitching(false)
    if (session.current !== current) {
      if (outcome.kind !== 'lost') outcome.capture.release()
      return
    }
    if (outcome.kind === 'lost') {
      lose(CAMERA_LOST)
      return
    }
    if (outcome.kind === 'kept') setError('다른 카메라로 바꾸지 못했습니다.')
    current.capture = outcome.capture
    setFacing(outcome.capture.facing)
    show(outcome.capture)
  }

  // Runs after every render, so a page hidden while connecting pauses once it publishes, and a
  // return that raced the paused render still resumes.
  useEffect(() => {
    const sync = () => {
      const hidden = document.visibilityState === 'hidden'
      const pausedMs = phase.kind === 'paused' ? Date.now() - phase.since : 0
      const action = backgroundAction({ phase: phase.kind, hidden, pauses, status: broadcast.status, pausedMs })
      if (action === 'pause') pause()
      else if (action === 'resume') void start()
      else if (action === 'idle') setPhase({ kind: 'idle' })
    }
    sync()
    document.addEventListener('visibilitychange', sync)
    return () => document.removeEventListener('visibilitychange', sync)
  })

  const busy = phase.kind === 'capturing' || phase.kind === 'connecting' || phase.kind === 'stopping'
  const publishing = phase.kind === 'publishing'
  const choosing = phase.kind === 'idle' || phase.kind === 'paused'
  const onAir = broadcast.status === 'LIVE' || broadcast.status === 'INTERRUPTED'
  const canStart = supported && !publishing && (broadcast.status === 'READY' || onAir)
  const canStop = publishing || onAir
  const interrupted = broadcast.status === 'INTERRUPTED' && phase.kind === 'idle'
  const mirrored = source === 'camera' && facing === 'user'

  return (
    <section className="publisher" aria-label="송출">
      <div className="stage">
        <video
          ref={preview}
          className={mirrored ? 'preview mirrored' : 'preview'}
          autoPlay
          muted
          playsInline
          hidden={choosing}
        />
        {choosing && (
          <svg className="stage-icon" viewBox="0 0 24 24" aria-hidden="true">
            <rect x="2" y="5" width="15" height="14" rx="2" />
            <path d="m17 10 5-3v10l-5-3" />
          </svg>
        )}
      </div>
      <div className="publisher-panel">
        <p className="publish-state" role="status" data-phase={phase.kind}>
          {phase.kind === 'capturing' &&
            (source === 'camera' ? '카메라와 마이크 권한을 확인하는 중…' : '공유할 화면을 고르세요…')}
          {phase.kind === 'connecting' && '송출 서버에 연결하는 중…'}
          {phase.kind === 'publishing' && `송출 중${phase.stats ? ` · ${formatStats(phase.stats)}` : ''}`}
          {phase.kind === 'stopping' && '방송을 종료하는 중…'}
        </p>
        {!supported && <p className="error">이 브라우저에서는 송출할 수 없습니다. HTTPS 주소로 접속했는지 확인하세요.</p>}
        {sources.length > 1 && (
          <fieldset className="source-choice" disabled={!choosing}>
            <legend>송출 방식</legend>
            {sources.map((s) => (
              <label key={s}>
                <input type="radio" name="source" value={s} checked={source === s} onChange={() => setSource(s)} />
                {sourceLabel[s]}
              </label>
            ))}
          </fieldset>
        )}
        <div className="actions publisher-actions">
          {(canStart || !supported) && (
            <button type="button" className="button-primary" onClick={start} disabled={busy || !supported}>
              방송 시작
            </button>
          )}
          {canStop && (
            <button type="button" className="button-danger" onClick={stop} disabled={busy}>
              방송 종료
            </button>
          )}
          {supported && source === 'camera' && (canStart || publishing) && (
            <button type="button" onClick={flipCamera} disabled={busy || switching}>
              카메라 전환 <span className="facing">({facingLabel[facing]})</span>
            </button>
          )}
        </div>
        {pauses && choosing && canStart && (
          <p className="hint-text">
            방송 중에 휴대폰을 돌리면 화면 크기가 바뀌어 일부 시청자 화면이 멈출 수 있습니다. 방향을 정한 뒤 시작하세요.
          </p>
        )}
        {phase.kind === 'paused' && (
          <p className="notice">화면이 꺼지거나 다른 앱으로 전환되어 송출을 멈췄습니다. 30초 안에 돌아오면 자동으로 이어집니다.</p>
        )}
        {interrupted && <p className="notice">연결 끊김, 30초 안에 다시 시작하면 이어집니다.</p>}
        {notice && <p className="notice">{notice}</p>}
        {error && (
          <p className="error" role="alert">
            {error}
          </p>
        )}
      </div>
    </section>
  )
}
