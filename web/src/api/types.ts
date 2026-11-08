// Hand-written until issue #6 generates these from the OpenAPI document.

export type User = {
  id: string
  username: string
}

export type Credentials = {
  username: string
  password: string
}

export type DemoAccount = 'streamer' | 'viewer'

export type DemoStatus = {
  enabled: boolean
}

export type ErrorBody = {
  code: string
  message: string
  traceId: string
}
