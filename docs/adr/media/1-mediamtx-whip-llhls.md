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
