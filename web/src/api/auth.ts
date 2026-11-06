import { request } from './client.ts'
import type { Credentials, DemoAccount, DemoStatus, User } from './types.ts'

export const authApi = {
  me: () => request<User>('GET', '/api/auth/me'),
  signup: (credentials: Credentials) => request<User>('POST', '/api/auth/signup', credentials),
  login: (credentials: Credentials) => request<User>('POST', '/api/auth/login', credentials),
  logout: () => request<void>('POST', '/api/auth/logout'),
  demoStatus: () => request<DemoStatus>('GET', '/api/auth/demo'),
  demoLogin: (account: DemoAccount) => request<User>('POST', '/api/auth/demo-login', { account }),
}
