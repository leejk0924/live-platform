import type { User } from '../api/types.ts'

/** 방송을 만들 수 있는 계정인지. 화면 안내용이고 실제 인가는 서버가 한다. */
export function canCreateBroadcast(user: User | undefined): boolean {
  return user?.role === 'streamer'
}
