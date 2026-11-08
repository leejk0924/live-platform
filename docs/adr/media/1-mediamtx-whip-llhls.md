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

### 실기기 결과

PC는 Windows의 Chrome 156.0.8078.8, 휴대폰은 iPhone 15 Pro(iOS 26.6.2) Safari. 스택은 PC 한 대에 띄웠다. 두 기기는 같은 Wi-Fi가 아니라 Tailscale로 연결했고, 사이트 주소와 ICE 후보 주소를 Tailscale 주소로 바꿨다. 결과는 사용자가 실기기에서 확인했다.

| # | 항목 | 기기 / 브라우저 | 결과 |
|---|---|---|---|
| 1 | PC 화면 + 탭 소리 + 마이크 → WHIP | PC Chrome → 다른 기기 WHEP | 통과. 화면, 탭 소리, 마이크 모두 들림 |
| 2 | Android Chrome 카메라 → WHIP | - | 미확인. Android 기기 없음 |
| 3 | iOS Safari 카메라 → WHIP | iPhone → PC Chrome WHEP | 통과. `sent.video` 29fps, 1690kbps |
| 4 | iOS Safari 내장 LL-HLS Opus 오디오 | iPhone Safari | **실패**. `error 3 Media failed to decode`(`MEDIA_ERR_DECODE`) |
| 5 | iOS Safari WHEP | iPhone Safari | 통과. 영상과 소리 |
| 6 | 송출 720p30 / 2.5Mbps 상한 | PC Chrome 화면 공유 | 상한 안. 높이 680, 약 20fps, 약 357kbps. 비트레이트 상한에는 닿지 않음 |

## Follow-up (2026-11-08, #3 실기기 결과에 따른 결정)

- **iOS Safari 시청은 WHEP로 정한다.** 4번이 실패했고 같은 기기에서 같은 송출을 WHEP로는 재생했다. 데이터는 도착했고 디코딩에서 실패한 것이라 네트워크 문제가 아니다. API는 `playback`에 두 방식을 모두 내려주고, #8 `Player`가 기기별 우선순위로 고른다.
- **브라우저 내장 HLS에 기대지 않는다.** Chrome 내장 HLS(`DEMUXER_ERROR_COULD_NOT_PARSE`)와 iOS Safari 내장 HLS(`MEDIA_ERR_DECODE`) 둘 다 이 스트림을 재생하지 못했다. `canPlayType`은 판단 근거가 되지 못한다. LL-HLS는 hls.js(MSE) 경로로만 쓴다.
- 4번 실패의 원인은 구분하지 못했다. 후보는 둘이다.
  - A. LL-HLS part 길이 흔들림. 테스트 중 MediaMTX가 `part duration changed ... this will cause an error in iOS clients` 경고를 남겼다(로그로 확인). (unverified) WebRTC 입력은 프레임 간격이 고르지 않아 생기는 것으로 추정한다.
  - B. (unverified) iOS 내장 플레이어가 fMP4 안의 Opus를 디코딩하지 못한다.
  - 구분하려면 MediaMTX HLS를 일반 fMP4 HLS(`hlsVariant: fmp4`)로 바꿔 iPhone에서 다시 본다. 재생되면 A, 계속 실패하면 B다. A라면 iOS도 LL-HLS로 되돌릴 수 있다.

### Cautions when touching this (추가)

- iOS 시청자는 WHEP라서 CDN으로 확장되지 않는다. 시청자 수 계획에서 iOS 몫은 MediaMTX WebRTC 용량으로 계산한다.
- (unverified) iOS 17.1 이상의 `ManagedMediaSource`로 hls.js를 iOS에서 돌리는 길은 확인하지 않았다. iOS를 LL-HLS로 되돌리려면 위 A/B 실험과 함께 이것부터 본다.
- 6번은 상한이 걸려 있다는 것만 확인했다. 실제 상한에 닿았을 때의 동작은 움직임이 많은 화면으로 다시 재야 한다.
- Android 송출(2번)은 확인하지 않았다. #9(모바일 송출) 전에 확인한다.

## Follow-up (2026-11-08, #7 송출 구현)

"재연결 시 토큰 재발급 거부"만으로는 재송출을 막지 못한다. 송출 토큰이 12시간 유효하고, MediaMTX는 DB 상태를 보지 않기 때문이다. kick 직후에도 같은 토큰으로 다시 송출할 수 있다. 이 공백은 #10의 publish hook으로 막는다. 결정과 이유는 [media/7-whip-publish-jwt](7-whip-publish-jwt.md).

## Follow-up (2026-11-08, 코드 대조)

- adapter는 `MediaMtxAdapter` 하나가 아니라 셋으로 나뉘었다: `MediaMtxIngestRouter`(`IngestRouter`), `MediaMtxControlApi`(`StreamTerminator`, `StreamProbe`), `JwtPublishAuthorizer`(`PublishAuthorizer`, 송출 허가 검증).
- 공용 `FakeMediaGateway`는 없고, 테스트가 port마다 Fake를 둔다. 교체 가능성은 port 단위 Fake로 증명한다.
- `PlaybackResolver`는 아직 없다. #8(플레이어)에서 만든다.

## Follow-up (2026-11-09, #8 시청)

`PlaybackResolver`를 `MediaMtxPlaybackResolver`로 만들었다. 프론트 `Player`는 이 ADR의 결정대로 내장 HLS를 쓰지 않고 hls.js로 LL-HLS를 재생하며, iOS는 WHEP로 본다. 결정과 이유는 [media/8-playback-player](8-playback-player.md).

## Follow-up (2026-11-09, #12 녹화)

녹화는 MediaMTX fMP4 세그먼트를 API가 `RecordingStorage`(로컬·S3)로 옮기고, 다시보기는 5분짜리 서명 URL로 일반 `<video>`에서 재생한다. CloudFront 서명은 RSA가 아니라 ECDSA P-256이다. 결정과 이유는 [recording/12-recording-replay](../recording/12-recording-replay.md).
