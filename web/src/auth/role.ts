import type { User } from '../api/types.ts'
import type { AuthState } from './context.ts'

/** 방송을 만들 수 있는 계정인지. 화면 안내용이고 실제 인가는 서버가 한다. */
export function canCreateBroadcast(auth: AuthState): auth is { status: 'signedIn'; user: User } {
  return auth.status === 'signedIn' && auth.user.role === 'streamer'
}
