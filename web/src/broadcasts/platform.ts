// userAgentData is Chromium only and missing from the DOM types.
export type Platform = Pick<Navigator, 'userAgent' | 'platform' | 'maxTouchPoints'> & {
  userAgentData?: { mobile: boolean }
}

// iPadOS reports itself as a Mac, so a Mac with a touch screen counts as iOS.
export function isIos(nav: Platform = navigator): boolean {
  return /iPhone|iPad|iPod/.test(nav.userAgent) || (nav.platform === 'MacIntel' && nav.maxTouchPoints > 1)
}

// Safari and Firefox do not send userAgentData, so they are recognized by their user agent.
export function isMobile(nav: Platform = navigator): boolean {
  return nav.userAgentData?.mobile === true || isIos(nav) || /Android/i.test(nav.userAgent)
}
