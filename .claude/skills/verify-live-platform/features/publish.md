# Go live (desktop screen or phone camera)

The owner of a READY broadcast presses `방송 시작` on its page. The browser publishes over WHIP to MediaMTX, the server moves the broadcast to LIVE, and viewers can watch. Desktop shares the screen. A phone, detected by UA through `isMobile`, uses the camera and microphone and can switch between the front and rear cameras.

## Sub-features
- Desktop: `getDisplayMedia` screen share, publisher status `송출 중 · <kbps> · <WxH>`.
- Phone: camera and microphone, `카메라 전환` before and during publishing (label `.facing`), mirrored preview for the front camera.
- Background on a phone:
  - Hiding the page pauses publishing. The broadcast turns INTERRUPTED after about 6 s.
  - Coming back within 30 s resumes publishing automatically, and the broadcast turns LIVE.
  - Past 30 s the broadcast turns ENDED and the camera stays off.
  - A desktop keeps publishing while hidden.
- Layout: no horizontal scroll at 412x915 and 390x844 in portrait and landscape. Publisher buttons are at least 44 px tall.
- A refresh failure on the detail page keeps the publisher mounted, so the WHIP session survives (#8 F10).

## How to get to it (user POV)
1. Log in.
2. Open `/broadcasts/new` and create a broadcast. Its page shows `방송 시작`.
3. Press it, then `방송 종료` to end.

## Driving it with Playwright
`bash scripts/run.sh publish` ([journeys/publish.mjs](../scripts/journeys/publish.mjs)), about 1.5 minutes:
1. Phone: `phoneContext(browser, 'phone', 'Pixel 7')` (Playwright device, `getDisplayMedia` removed, three fake cameras with `fake_device_1` hidden).
   - Expect no `.source-choice`, no horizontal scroll at 412 px, and publisher buttons at least 44 px tall.
   - `카메라 전환` before going live flips `.facing` (`전면 카메라` ↔ `후면 카메라`).
2. `방송 시작`. Wait for `.publish-state` to contain `kbps` and the API status to be `LIVE`. An anonymous desktop viewer plays LL-HLS (`assertPlaying`).
3. Live switch: press `카메라 전환` twice, 4 s apart. Each time expect a new `.facing`, no `.error`, `송출 중`, status `LIVE`, and the viewer's `currentTime` advancing over 3 s.
4. Layout: `setViewportSize(915x412)`, then expect `scrollWidth - innerWidth === 0`.
5. Background: `setHidden(page, true)` overrides `document.visibilityState` and dispatches `visibilitychange`.
   - Expect `송출을 멈췄습니다` and `INTERRUPTED`. Unhide, then expect `송출 중` and `LIVE`.
   - Hide again and expect `INTERRUPTED`, then `ENDED` after the 30 s grace. Unhide, and the preview has no live video track.
6. Desktop: the same account publishes the screen (`startPublishing`), hides the tab for 10 s, and is still `LIVE` and `송출 중`. Then `방송 종료` gives `ENDED`.

End state that proves it: status transitions `READY → LIVE → INTERRUPTED → LIVE → INTERRUPTED → ENDED` from the API, a viewer whose video advances across camera switches, a desktop that keeps publishing while hidden, and no CSP violations.

## Gotchas
- Chrome's fake cameras report no `facingMode`, so the journey exercises the switch's fallback (the next camera by deviceId), not the `facingMode: exact` path that real phones take. `fake_device_1` is hidden because WebRTC encodes none of its frames.
- Real Android and iOS devices are unverified (#9 stays open for this). That covers the real camera, screen lock, app switching, rotation, and iOS resuming without a user gesture.
- Rotating a phone mid-broadcast changes the resolution. MediaMTX does not negotiate CVO, so LL-HLS viewers who are already watching can freeze. The publisher shows a hint. Do not rotate in a journey and expect LL-HLS to keep playing.
- Android "desktop site" mode is detected as a PC (#9 F7).
- Publishing needs a secure context. `https://localhost` works, `http://` does not.
