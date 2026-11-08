import { api, unwrap } from './client.ts'
import type { Credentials, DemoAccount } from './types.ts'

export const authApi = {
  me: () => unwrap(api.GET('/api/auth/me')),
  signup: (credentials: Credentials) => unwrap(api.POST('/api/auth/signup', { body: credentials })),
  login: (credentials: Credentials) => unwrap(api.POST('/api/auth/login', { body: credentials })),
  logout: () => unwrap(api.POST('/api/auth/logout')),
  demoStatus: () => unwrap(api.GET('/api/auth/demo')),
  demoLogin: (account: DemoAccount) => unwrap(api.POST('/api/auth/demo-login', { body: { account } })),
}
