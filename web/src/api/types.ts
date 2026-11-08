import type { components } from './schema.d.ts'

type Schemas = components['schemas']

export type User = Schemas['User']
export type Credentials = Schemas['Credentials']
export type DemoAccount = Schemas['DemoAccount']
export type Broadcast = Schemas['Broadcast']
export type BroadcastInput = Schemas['BroadcastInput']
export type BroadcastStatus = Schemas['BroadcastStatus']
export type PlaybackSource = Schemas['PlaybackSource']
export type PlaybackKind = Schemas['PlaybackKind']
