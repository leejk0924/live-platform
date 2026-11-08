import type { PlaybackSource } from '../../api/types.ts'

export type Device = { ios: boolean; hlsSupported: boolean }

// The server lists sources in its preferred order. Native HLS is never used: Chrome and iOS Safari
// both failed to decode this stream (ADR media/1), so LL-HLS means hls.js, and iOS gets WHEP.
export function playOrder(
  sources: PlaybackSource[],
  { ios, hlsSupported, lowLatency }: Device & { lowLatency: boolean },
): PlaybackSource[] {
  const usable = sources.filter((s) => s.kind === 'whep' || (hlsSupported && !ios))
  if (!ios && !lowLatency) return usable
  return [...usable.filter((s) => s.kind === 'whep'), ...usable.filter((s) => s.kind !== 'whep')]
}

// iPadOS reports itself as a Mac, so a Mac with a touch screen counts as iOS.
// hls.js may claim support on iOS through ManagedMediaSource, which is unverified here.
export function isIos(nav: Navigator = navigator): boolean {
  return /iPhone|iPad|iPod/.test(nav.userAgent) || (nav.platform === 'MacIntel' && nav.maxTouchPoints > 1)
}
