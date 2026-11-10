# Recording and replay

Every live broadcast is recorded by MediaMTX in 30-minute fMP4 segments. When a segment completes, a hook registers it. The api moves the file from the spool to recording storage in the background, and the recording turns READY. Viewers find recordings at `/recordings` and on the broadcast page, and play them through a signed URL.

## Sub-features
- `recordings` rows go `PENDING → READY`, or `FAILED`, with `storage_key`, `duration_ms` and `size_bytes`.
- Local storage serves files through an HMAC-signed URL (`/api/recordings/{id}/file?expires=…&signature=…`). A forged signature gives 403.
- Deleting the broadcast makes the replay URL, already-issued file URLs and the detail endpoint return 404.
- Replay supports seeking: Range requests through Caddy get 206 with no compression. After a URL fails, the player fetches a fresh one and resumes.

## How to get to it (user POV)
- Open `/recordings` (`section.feed[aria-label="녹화 목록"]`), or the `다시보기` list on a broadcast page, and click a recording.
- The page `/recordings/<id>` shows `section.player[aria-label="다시보기"]`.

## Driving it with Playwright
`bash scripts/run.sh recording` ([journeys/recording.mjs](../scripts/journeys/recording.mjs)):
1. The owner (role `recorder`) goes live (`startPublishing`), waits 10 s, and presses `방송 종료`. The end of the broadcast closes the segment.
2. Poll `GET /api/recordings?broadcastId=<id>` for up to 60 s. Expect `READY`, `durationMs >= 8000` and `sizeBytes > 0`.
3. Side effect: `docker compose cp api:/recording-store` (the api image is distroless, so `exec ls` fails). Expect exactly one `recordings/<broadcast>/<recording>.mp4` of the reported size. The journey keeps that file as `recording.mp4` in the evidence and drops the rest of the copy.
4. An anonymous viewer finds the title in `/recordings` (`section.feed[aria-label="녹화 목록"]`) and clicks it. In `section.player[aria-label="다시보기"]` the `video` plays, and after `currentTime = 4` it continues past 4 s.
5. The signed file URL from `/api/recordings/<id>/replay` answers 206 to a `Range` request. The same URL with one character of `signature` changed gets 403.
6. The owner deletes the broadcast (204). The replay endpoint and the already-issued file URL both get 404.

End state that proves it: the row is READY, the file is present, the video plays and seeks, the forged URL gets 403, and both URLs get 404 after delete.

## Gotchas
- Segments are 30 minutes long, so a short journey gets its recording only after `방송 종료`. Do not wait for READY while still LIVE.
- S3, CloudFront and IAM are not reachable locally. S3 is proven only against a fake server in tests. HTTPS for S3 calls also needs the TLS that #13 turned on, and that path has never run against real AWS.
- The local signing key changes on api restart (#12 F14). A replay open across a relaunch fails once and then refetches. That is expected.
- Keep test recordings short. Each takes disk space in the volume.
