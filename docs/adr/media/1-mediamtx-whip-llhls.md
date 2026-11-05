---
issue: 1
area: media
summary: 미디어 중계는 MediaMTX(WHIP 송출, LL-HLS 기본·WHEP 옵션 시청)에 맡기고 Rust는 control plane만 담당
status: confirmed
date: 2026-11-05
paths: [crates/application/src/ports/media.rs, crates/infrastructure/src/media]
related: []
---

# 스트리밍 아키텍처

## Background

PC 화면 공유(탭/시스템 소리 + 마이크)와 모바일 카메라 방송을 다른 기기에서 실시간 시청해야 한다. 7일 일정, 피크 500만 시청 방향성.

## Decision and reasons

- **Rust는 control plane**(인증, 방송 상태, 권한)만. 미디어 중계는 검증된 MediaMTX에 맡긴다. Rust의 성능 이점은 API·동시성 영역에서 가져간다.
- **송출: WHIP(WebRTC)**. 브라우저 표준 API만으로 PC·모바일 모두 송출 가능.
- **시청: LL-HLS 기본, WHEP 옵션**. 대규모는 CDN이 가능한 LL-HLS만 확장된다. API는 `playback: [{kind, url}]`로 둘 다 내려주고 프론트 `Player`가 고른다.
- **트랜스코딩 없음(passthrough)**. 대신 송출 측 `RTCRtpSender.setParameters`로 720p30 / 2.5Mbps 제한. `Transcoder` port는 ADR로만.
- **port 4개(ISP)**: `IngestRouter`(노드 배정), `PublishAuthorizer`(JWT 발급), `PlaybackResolver`, `StreamTerminator`(kick). `MediaMtxAdapter`가 3개, `JwtPublishAuthorizer`가 1개 구현. 테스트용 `FakeMediaGateway`가 두 번째 구현체로 교체 가능성을 증명.
- **종료 순서**: DB CAS(→ENDED) 커밋 → MediaMTX Control API kick → 재연결 시 토큰 재발급 거부.
- **비정상 종료**: unpublish hook → `INTERRUPTED`, 30초 내 재송출 시 `LIVE` 복귀, 초과 시 reaper(advisory lock)가 `ENDED`. reaper는 MediaMTX 경로 생존도 확인해 hook 유실을 보완.
- **녹화**: MediaMTX 녹화 → `RecordingStorage`(S3) → CloudFront 서명 URL.

## Rejected alternatives

- webrtc-rs로 SFU 직접 구현: 7일 내 불가에 가까운 리스크.
- MediaRecorder → WebSocket → ffmpeg → HLS: 지연 5~10초, ffmpeg 프로세스 관리 부담.
- WHEP만으로 시청: SFU 노드 수천 대 필요, 대규모 불가.
- LiveKit 지금 구현: Room/SDK 모델이 프론트까지 바뀜. trait는 LiveKit을 수용하도록(`PublishCredential` enum) 설계만.
- SRS: 버퍼 기간에 두 번째 실제 adapter로 고려.

## Cautions when touching this

- (unverified) iOS Safari가 Opus 오디오가 담긴 LL-HLS를 재생하는지 1일차 실기기 검증 필요. 실패 시 iOS는 WHEP fallback.
- 탭/시스템 소리 캡처는 Chromium 계열에서만 지원. README 지원 환경에 명시.
- MediaMTX Control API는 외부에 노출하지 않는다.

## Follow-up (2026-11-06, #3 미디어 PoC)

송출·시청 페이지와 절차는 [`docs/poc/3-media-device-check.md`](../../poc/3-media-device-check.md).

### 로컬에서 확인한 것 (headless Chrome, 가짜 카메라·화면, `https://localhost`와 LAN IP 둘 다)

- WHIP 송출 → MediaMTX 경로 트랙 `H264`(Baseline, 1280x720) + `Opus`. 화면 소스는 탭 오디오와 마이크 2개를 Web Audio로 합쳐 오디오 트랙 1개로 보낸다.
- `setParameters` 후 `getParameters`에 `maxBitrate: 2500000`, `maxFramerate: 30`이 남는다. 가짜 소스는 300~450kbps라 상한에 닿는지는 실기기 화면 공유로 본다.
- WHEP 재생과 MediaMTX hls.js 플레이어(LL-HLS, Opus 오디오 디코딩 확인)는 동작한다.
- LL-HLS 마스터 플레이리스트는 오디오를 별도 rendition으로 내고 `CODECS="avc1.42401f,opus"`를 알린다.
- **Chrome 내장 HLS는 `canPlayType`이 `maybe`인데 이 스트림을 `DEMUXER_ERROR_COULD_NOT_PARSE`로 거부한다.** 원인은 확인하지 않았다(추정: Opus in fMP4). 따라서 프론트 `Player`는 `canPlayType`만으로 내장 HLS를 고르면 안 된다.
- Caddy가 IP 주소로 서빙할 때는 클라이언트가 SNI를 보내지 않아 TLS가 실패했다. `default_sni`를 사이트 주소로 둬 해결했다.

### 실기기 결과 (사용자가 채운다)

| # | 항목 | 기기 / 브라우저 | 결과 |
|---|---|---|---|
| 1 | PC 화면 + 탭 소리 + 마이크 → WHIP | | |
| 2 | Android Chrome 카메라 → WHIP | | |
| 3 | iOS Safari 카메라 → WHIP | | |
| 4 | iOS Safari 내장 LL-HLS Opus 오디오 | | |
| 5 | iOS Safari WHEP | | |
| 6 | 송출 720p30 / 2.5Mbps 상한 | | |

4번이 실패하면 iOS 시청은 WHEP로 정하고 `playback` 우선순위를 기기별로 내려준다.
