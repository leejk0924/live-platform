---
issue: 8
area: media
summary: 시청 주소는 방송 중(LIVE·INTERRUPTED)일 때만 순서 있는 목록으로 내리고, 프론트는 내장 HLS 없이 hls.js LL-HLS를 기본으로 iOS·저지연은 WHEP로, 실패하면 다음 소스로 넘어간다
status: confirmed
date: 2026-11-09
paths: [crates/application/src/playback.rs, crates/application/src/ports/media.rs, crates/infrastructure/src/media/mediamtx.rs, crates/api/src/live.rs, web/src/broadcasts/watch, web/src/broadcasts/broadcastState.ts, web/src/pages/BroadcastDetail.tsx, web/src/pages/Home.tsx]
related: [1, 7, 10]
---

# 시청: playback API와 Player

## Background

로그인하지 않은 시청자도 다른 기기에서 방송을 실시간으로 봐야 한다. media/1은 LL-HLS를 기본, WHEP를 옵션으로 정했고, 실기기 PoC(#3)에서 브라우저 내장 HLS가 Chrome과 iOS Safari 모두 이 스트림을 디코딩하지 못했다. 이슈 문구의 "Safari 네이티브"는 이 결과로 대체한다. 다시보기와 시청자 수 집계는 범위 밖이다.

## Decision and reasons

| Decision | Reason |
|---|---|
| `PlaybackResolver::resolve(&StreamPath) -> Vec<PlaybackSource{kind,url}>`, 순서가 곧 기본 선호 | 경로 이름은 `IngestRouter` 한 곳에서만 정한다. 순서를 서버가 쥐면 기본 방식을 프론트 배포 없이 바꿀 수 있다 |
| LL-HLS가 첫 번째 | CDN으로 확장되는 것은 LL-HLS뿐이고, WHEP 시청자는 미디어 서버에 WebRTC 세션을 하나씩 잡는다(media/1) |
| `GET /api/broadcasts/{id}/playback`은 인증 없음, LIVE·INTERRUPTED만 200, READY·ENDED는 409 `BROADCAST_NOT_LIVE`, 삭제·없음은 404 | 시청은 MediaMTX에서도 인증이 없다(media/10). "종료된 방송은 재생하지 않는다"를 서버 규칙으로 둔다 |
| 응답 `Cache-Control: public, max-age=5` | 목록과 같은 이유. 모든 시청자에게 같고, 몇 초 늦게 아는 것만 대가다 |
| 목록 `live=true|false`를 `ListFilter{All,OnAir,OffAir}`로, SQL은 domain `ACTIVE`를 바인딩 | 홈을 "방송 중"과 나머지로 나눌 때, 클라이언트에서만 묶으면 첫 페이지 밖의 오래된 LIVE 방송이 빠진다 |
| 브라우저 내장 HLS와 `canPlayType`을 쓰지 않는다. LL-HLS는 hls.js(MSE)로만 | PoC에서 내장 HLS가 Chrome(`DEMUXER_ERROR_COULD_NOT_PARSE`), iOS(`MEDIA_ERR_DECODE`) 모두 실패했다 |
| iOS는 UA로 판별해 WHEP만. `Hls.isSupported()`를 믿지 않는다 | iOS WHEP는 실기기로 통과했다. ManagedMediaSource 경로는 확인하지 않았다 |
| `playOrder`(순수 함수)가 순서를 정하고 Player는 `loading | playing | failed` 상태로 소스를 차례로 시도한다. 모두 실패하면 페이지가 상태를 다시 묻는다 | 기기별 규칙을 한 곳에 모으고 단위 테스트로 고정한다 |
| hls.js 매니페스트 404는 다섯 번까지 다시 시도한다 | MediaMTX는 방송이 LIVE가 된 직후 잠시 404를 준다. 바로 포기하면 초기 시청자가 모두 WHEP로 몰린다(e2e에서 확인) |
| WHEP는 피어가 `connected`가 된 뒤에만 재생으로 보고, POST와 연결을 합쳐 10초 기한을 둔다. `AbortSignal.any`는 쓰지 않는다 | ICE가 막히면 수십 초 검은 화면이 된다. iOS 17.4 전에는 `AbortSignal.any`가 없고 iOS는 WHEP뿐이다 |
| 상세 화면은 앞 응답이 온 뒤 5초 뒤에 다시 묻고, 새로 고침이 실패해도 보던 화면을 유지한다(`refreshFailed`). 첫 로드 실패만 오류 화면 | 고정 주기면 느린 망의 응답이 늘 버려진다. 한 번의 실패로 화면이 바뀌면 소유자 Publisher가 언마운트되어 송출이 끊긴다 |
| 웹 단위 테스트는 의존성 없이 Node 내장 `node --test`(타입 제거) | 테스트 러너를 들이지 않고 순수 규칙과 WebRTC 헬퍼를 고정한다 |

## Rejected alternatives

- 클라이언트에서만 상태별로 묶기: 페이지네이션 밖의 방송 중 항목이 빠진다.
- 브라우저 내장 HLS 우선(이슈 문구): 실기기에서 재생되지 않았다.
- `live=true`용 부분 인덱스 추가(리뷰 F3): 이슈 완료 조건 밖이다. 지금 쿼리는 `id DESC` 부분 인덱스를 따라가며 상태로 거른다. 후속 이슈 후보다.
- INTERRUPTED 동안 Player 유지(리뷰 F6): 그동안 경로에 송출자가 없어 모든 소스가 실패한다. 안내만 보이고, LIVE로 돌아오면 Player를 다시 마운트한다.

## Cautions when touching this

- Player의 `onLost`는 안정된 함수여야 한다. effect 의존성이라 바뀔 때마다 재생이 다시 시작된다.
- hls.js 청크는 약 580kB다. 오디오가 별도 rendition이라 `hls.js/light`로 줄일 수 없다(소스 확인은 서브에이전트, 이 세션 미확인).
- (unverified) 실제 iPhone에서의 WHEP 시청은 #3 PoC 페이지로만 확인했다. 이번 Player는 iPhone UA 에뮬레이션(Chromium)으로만 확인했다.
- (unverified) INTERRUPTED에서 LIVE로 돌아올 때 Player가 다시 붙는 경로는 실행해 보지 않았다.
- WHEP POST가 201 전에 끊기면 지울 세션 주소가 없다. MediaMTX가 핸드셰이크 시간 초과로 정리한다.
- 편집 화면도 `useBroadcast`를 쓴다. 충돌 뒤 다시 불러오기가 실패하면 이제 오류 화면 대신 이전 값을 유지한다(안내 문구 없음).
