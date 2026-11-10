# Watch a live broadcast

Anyone, signed in or not, finds a live broadcast on the home page and watches it. LL-HLS through hls.js is the default. WebRTC (WHEP) is used on request and always on iOS. When the owner ends the broadcast, viewers get an end notice instead of a player.

## Sub-features
- Home splits `방송 중` (LIVE, INTERRUPTED) from `지난·예정 방송` (READY, ENDED), each with its own paging.
- `GET /api/broadcasts/{id}/playback` works without a cookie and returns `[ll-hls, whep]` in that order. It returns 409 `BROADCAST_NOT_LIVE` for READY and ENDED, and 404 for deleted broadcasts.
- The player falls through to the next source on fatal errors. A WHEP source counts as playing only once the peer connects, and it gives up after 10 s.
- The low-latency toggle `저지연(WebRTC)` switches to WHEP. iOS gets WHEP directly and has no toggle.
- The detail page re-polls about every 5 s. On ENDED it shows `종료된 방송입니다.` and removes the `<video>`.

## How to get to it (user POV)
Open `https://localhost/` and click a title under `방송 중`, or open `/broadcasts/<id>` directly. No login is needed.

## Driving it with Playwright
`bash .claude/skills/verify-live-platform/scripts/run.sh watch` ([scripts/journeys/watch.mjs](../scripts/journeys/watch.mjs)). The journey runs these steps:
1. Owner signs up, creates a broadcast, presses `방송 시작`. The desktop publisher shares the fake screen. Wait for `.publish-state` `송출 중` and API status `LIVE`.
2. Run `curl -k /api/broadcasts/<id>/playback` without a cookie. Expect 200 with `ll-hls,whep`.
3. A fresh context with no cookies finds the title in `section.feed[aria-label="방송 중"]` and clicks it. `section.player[data-source="ll-hls"]` appears, and `video.currentTime` advances over 3 s.
4. Check `저지연(WebRTC)`. `data-source="whep"` appears, and the video advances.
5. An emulated iPhone 15 opens the page. The player shows `whep` and no toggle.
6. Owner presses `방송 종료`. Status turns `ENDED`, both viewers show `종료된 방송입니다.`, and no `<video>` remains. Home lists the title under `지난·예정 방송`.
7. No CSP violations on any page.

End state that proves it: `VERIFY OK (19 checks)`, with screenshots `1-owner-publishing` through `7-home-ended`.

## Gotchas
- iOS is UA emulation in desktop Chrome, not Safari. Real iPhone playback is unverified. Native HLS is deliberately unused (ADR media/1), so emulation exercises the same code path.
- The first LL-HLS manifest 404s for a moment after LIVE. hls.js retries manifest 404s up to 5 times, so a single 404 in the log is expected.
- INTERRUPTED shows a notice, not the player (ADR media/8). This journey does not cover INTERRUPTED → LIVE remounting. Hiding the publisher tab past the grace period (see publish.md) is the way to drive it.
