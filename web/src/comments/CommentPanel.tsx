import { useEffect, useLayoutEffect, useReducer, useRef, useState, type FormEvent } from 'react'
import { Link } from 'react-router'
import { ApiError, errorMessage } from '../api/client.ts'
import { commentApi, eventsUrl } from '../api/comments.ts'
import type { Broadcast, ChatPolicy, Comment } from '../api/types.ts'
import { useAuth } from '../auth/context.ts'
import { startEdit, type EditDraft } from './commentEdit.ts'
import { commentsReducer, initialComments, visibleComments, type CommentsAction } from './commentsState.ts'

const MAX_CHARS = 500
const SLOW_MODE_CHOICES = [0, 3, 5, 10, 30, 60, 120, 300]
const NEAR_BOTTOM_PX = 48

// `closed`: the browser gave up on the stream (an error answer such as TOO_MANY_STREAMS); network
// drops are retried by EventSource itself and never get here.
type Feed = { status: 'loading' } | { status: 'live' } | { status: 'closed' } | { status: 'failed'; error: unknown }

function commentError(error: unknown): string {
  if (!(error instanceof ApiError)) return errorMessage(error)
  if (error.code === 'VERSION_CONFLICT') return '다른 곳에서 댓글이 수정되었습니다. 바뀐 댓글을 확인하고 다시 시도하세요.'
  if (error.code === 'RATE_LIMITED' && error.retryAfter !== undefined) {
    return `댓글을 너무 자주 쓰고 있습니다. ${error.retryAfter}초 후 다시 쓸 수 있습니다.`
  }
  return errorMessage(error)
}

export default function CommentPanel({ broadcast }: { broadcast: Broadcast }) {
  const { id } = broadcast
  const { auth } = useAuth()
  const [state, dispatch] = useReducer(commentsReducer, initialComments)
  const [feed, setFeed] = useState<Feed>({ status: 'loading' })
  const [attempt, setAttempt] = useState(0)

  // List first, then follow from the revision the list was read at, so nothing falls in between.
  // A `reset` means the server could not replay what this tab missed: load the list again.
  useEffect(() => {
    let active = true
    let source: EventSource | undefined
    Promise.all([commentApi.list(id), commentApi.getPolicy(id)]).then(
      ([page, policy]) => {
        if (!active) return
        dispatch({ type: 'loaded', items: page.items, slowModeSeconds: policy.slowModeSeconds })
        setFeed({ status: 'live' })
        const stream = new EventSource(eventsUrl(id, page.revision))
        source = stream
        stream.addEventListener('comments', (e: MessageEvent<string>) => {
          dispatch({ type: 'changed', items: JSON.parse(e.data) as Comment[] })
        })
        stream.addEventListener('policy', (e: MessageEvent<string>) => {
          dispatch({ type: 'policy', slowModeSeconds: (JSON.parse(e.data) as ChatPolicy).slowModeSeconds })
        })
        stream.addEventListener('reset', () => {
          stream.close()
          setAttempt((n) => n + 1)
        })
        stream.onerror = () => {
          if (stream.readyState === EventSource.CLOSED) setFeed({ status: 'closed' })
        }
      },
      (error: unknown) => active && setFeed({ status: 'failed', error }),
    )
    return () => {
      active = false
      source?.close()
    }
  }, [id, attempt])

  function retry() {
    setFeed({ status: 'loading' })
    setAttempt((n) => n + 1)
  }

  const isOwner = auth.status === 'signedIn' && auth.user.id === broadcast.ownerId
  const userId = auth.status === 'signedIn' ? auth.user.id : null
  const comments = visibleComments(state)

  return (
    <section className="comments" aria-label="댓글">
      <div className="comments-header">
        <h2>댓글</h2>
        {state.slowModeSeconds > 0 && <span className="slow-mode">슬로우 모드 {state.slowModeSeconds}초</span>}
      </div>
      {isOwner && broadcast.status !== 'ENDED' && (
        <SlowModeSelect broadcastId={id} value={state.slowModeSeconds} dispatch={dispatch} />
      )}
      {feed.status === 'failed' && (
        <p className="error" role="alert">
          댓글을 불러오지 못했습니다. {errorMessage(feed.error)}{' '}
          <button type="button" onClick={retry}>
            다시 시도
          </button>
        </p>
      )}
      {feed.status === 'closed' && (
        <p className="notice" role="status">
          실시간 댓글 연결이 끊겼습니다.{' '}
          <button type="button" onClick={retry}>
            다시 연결
          </button>
        </p>
      )}
      <CommentList
        comments={comments}
        loading={feed.status === 'loading'}
        broadcastId={id}
        userId={userId}
        dispatch={dispatch}
      />
      {broadcast.status === 'ENDED' ? (
        <p className="hint-text">종료된 방송은 댓글을 쓸 수 없습니다.</p>
      ) : auth.status === 'signedOut' ? (
        <p className="hint-text">
          댓글을 쓰려면{' '}
          <Link to="/login" state={{ from: `/broadcasts/${id}` }}>
            로그인
          </Link>
          하세요.
        </p>
      ) : (
        auth.status === 'signedIn' && <CommentForm broadcastId={id} dispatch={dispatch} />
      )}
    </section>
  )
}

type Dispatch = (action: CommentsAction) => void

function CommentList({
  comments,
  loading,
  broadcastId,
  userId,
  dispatch,
}: {
  comments: Comment[]
  loading: boolean
  broadcastId: string
  userId: string | null
  dispatch: Dispatch
}) {
  const listRef = useRef<HTMLOListElement>(null)
  const nearBottom = useRef(true)
  const newest = comments.at(-1)?.id

  // Follow new comments only while the reader is at the bottom; someone scrolled up to read stays put.
  useLayoutEffect(() => {
    const list = listRef.current
    if (list && nearBottom.current) list.scrollTop = list.scrollHeight
  }, [newest])

  function onScroll() {
    const list = listRef.current
    if (list) nearBottom.current = list.scrollHeight - list.scrollTop - list.clientHeight < NEAR_BOTTOM_PX
  }

  if (comments.length === 0) return <p className="hint-text">{loading ? '댓글을 불러오는 중…' : '아직 댓글이 없습니다.'}</p>
  return (
    <ol className="comment-list" ref={listRef} onScroll={onScroll}>
      {comments.map((comment) => (
        <CommentItem
          key={comment.id}
          comment={comment}
          broadcastId={broadcastId}
          mine={comment.authorId === userId}
          dispatch={dispatch}
        />
      ))}
    </ol>
  )
}

function CommentItem({
  comment,
  broadcastId,
  mine,
  dispatch,
}: {
  comment: Comment
  broadcastId: string
  mine: boolean
  dispatch: Dispatch
}) {
  const [draft, setDraft] = useState<EditDraft | null>(null)
  const [pending, setPending] = useState(false)
  const [error, setError] = useState<string | null>(null)

  async function run(work: () => Promise<void>) {
    setPending(true)
    setError(null)
    try {
      await work()
    } catch (e) {
      setError(commentError(e))
    } finally {
      setPending(false)
    }
  }

  function onSave(e: FormEvent) {
    e.preventDefault()
    if (draft === null) return
    void run(async () => {
      const saved = await commentApi.edit(broadcastId, comment.id, draft.version, draft.body)
      dispatch({ type: 'changed', items: [saved] })
      setDraft(null)
    })
  }

  function onDelete() {
    if (!window.confirm('이 댓글을 삭제할까요?')) return
    void run(async () => {
      await commentApi.remove(broadcastId, comment.id)
      dispatch({ type: 'deleted', id: comment.id })
    })
  }

  return (
    <li className="comment">
      <span className="comment-author">{comment.authorName}</span>
      {draft === null ? (
        <span className="comment-body">
          {comment.body}
          {comment.edited && <span className="comment-edited"> (수정됨)</span>}
        </span>
      ) : (
        <form className="comment-edit" onSubmit={onSave}>
          <input
            aria-label="댓글 수정"
            value={draft.body}
            maxLength={MAX_CHARS}
            onChange={(e) => setDraft({ ...draft, body: e.target.value })}
            disabled={pending}
          />
          <button type="submit" disabled={pending || draft.body.trim() === ''}>
            저장
          </button>
          <button type="button" onClick={() => setDraft(null)} disabled={pending}>
            취소
          </button>
        </form>
      )}
      {mine && draft === null && (
        <span className="comment-actions">
          <button type="button" onClick={() => setDraft(startEdit(comment))} disabled={pending}>
            수정
          </button>
          <button type="button" onClick={onDelete} disabled={pending}>
            삭제
          </button>
        </span>
      )}
      {error && (
        <span className="error comment-error" role="alert">
          {error}
        </span>
      )}
    </li>
  )
}

function CommentForm({ broadcastId, dispatch }: { broadcastId: string; dispatch: Dispatch }) {
  const [body, setBody] = useState('')
  const [pending, setPending] = useState(false)
  const [error, setError] = useState<string | null>(null)

  async function onSubmit(e: FormEvent) {
    e.preventDefault()
    setPending(true)
    setError(null)
    try {
      const posted = await commentApi.post(broadcastId, body)
      dispatch({ type: 'changed', items: [posted] })
      setBody('')
    } catch (e) {
      setError(commentError(e))
    } finally {
      setPending(false)
    }
  }

  return (
    <form className="comment-form" onSubmit={onSubmit}>
      <div className="comment-input">
        <input
          aria-label="댓글"
          placeholder="댓글을 입력하세요"
          value={body}
          maxLength={MAX_CHARS}
          onChange={(e) => setBody(e.target.value)}
        />
        <button type="submit" disabled={pending || body.trim() === ''}>
          등록
        </button>
      </div>
      <span className="comment-count" aria-live="polite">
        {body.length}/{MAX_CHARS}
      </span>
      {error && (
        <p className="error" role="alert">
          {error}
        </p>
      )}
    </form>
  )
}

function SlowModeSelect({ broadcastId, value, dispatch }: { broadcastId: string; value: number; dispatch: Dispatch }) {
  const [pending, setPending] = useState(false)
  const [error, setError] = useState<string | null>(null)

  async function onChange(seconds: number) {
    setPending(true)
    setError(null)
    try {
      const policy = await commentApi.setPolicy(broadcastId, seconds)
      dispatch({ type: 'policy', slowModeSeconds: policy.slowModeSeconds })
    } catch (e) {
      setError(commentError(e))
    } finally {
      setPending(false)
    }
  }

  return (
    <label className="slow-mode-select">
      슬로우 모드
      <select value={value} onChange={(e) => void onChange(Number(e.target.value))} disabled={pending}>
        {SLOW_MODE_CHOICES.map((seconds) => (
          <option key={seconds} value={seconds}>
            {seconds === 0 ? '끔' : `${seconds}초`}
          </option>
        ))}
      </select>
      {error && (
        <span className="error" role="alert">
          {error}
        </span>
      )}
    </label>
  )
}
