---
issue: 7
area: media
summary: 방송 시작·종료는 domain 상태 집합을 바인딩한 조건부 UPDATE로, 송출은 MediaMTX가 JWKS로 검증하는 EdDSA JWT로 연다. MediaMTX가 exp에 세션을 끊어 토큰 TTL은 12시간이다
status: confirmed
date: 2026-11-08
paths: [crates/application/src/live.rs, crates/application/src/ports/media.rs, crates/infrastructure/src/media, crates/api/src/live.rs, deploy/mediamtx/mediamtx.yml, web/src/broadcasts/publish]
related: [1, 3, 6, 10]
---

# PC 송출: 시작·종료 전이와 송출 JWT

## Background

방송 주인이 PC 화면, 탭 소리, 마이크를 송출하고 끝낼 수 있어야 한다. 설계는 auth/1(송출은 EdDSA 1회용 단기 JWT)과 media/1(WHIP 송출, MediaMTX 중계)을 따른다. INTERRUPTED 처리, unpublish hook, reaper는 #10의 범위라 이번에 만들지 않았다.

## Decision and reasons

- **전이 규칙은 domain 상수 하나에 둔다.** `STARTABLE=[READY]`, `STOPPABLE=[LIVE, INTERRUPTED]`를 SQL이 `status = ANY($n)`으로 바인딩한다. 조건부 UPDATE 하나로 상태를 바꾸고 `version`을 올린다. 0행일 때만 읽어서 404/403/409를 가린다. 삭제(#6)와 같은 방식이라 규칙이 SQL에 다시 적히지 않는다.
- **LIVE 방송에서 주인이 start를 다시 부르면 전이 없이 토큰만 새로 준다.** 방송 중 페이지를 새로고침해도 다시 송출할 수 있어야 해서다. 동시 start 50개는 전이 1회, 나머지는 재발급이다.
- **stop은 커밋 다음에 kick한다.** DB가 기준이다. kick이 실패하면 경고 로그만 남기고 200을 준다. 남은 송출은 #10 reaper가 정리한다. 클라이언트가 응답 전에 연결을 끊어도 kick이 빠지지 않도록 유스케이스를 별도 task에서 끝까지 실행한다. kick은 경로의 publisher 하나가 아니라 경로에 걸린 WebRTC 세션 전부를 끊는다. 아직 협상 중인 세션은 경로 source에 잡히지 않기 때문이다.
- **송출 인증은 MediaMTX `authMethod: jwt`이다.** API가 Ed25519로 서명하고 `/api/.well-known/jwks.json`으로 공개키를 낸다. `kid`는 공개키의 RFC 7638 thumbprint라 따로 설정하지 않는다. 권한은 `live/<id>` publish 하나다. 시청(read, playback)은 JWT에서 제외한다. `overridePublisher: false`로 송출 중인 경로를 같은 토큰이 빼앗지 못하게 한다.
- **MediaMTX Control API(kick)도 같은 키로 API가 자기 토큰(60초, `api` 권한)을 만들어 호출한다.** 별도 비밀값을 두지 않으려는 선택이다. 대신 healthcheck는 인증 없이 응답하는 WebRTC 리스너의 정적 파일로 바꿨다.
- **송출 토큰 TTL은 60초가 아니라 12시간이다.** MediaMTX 1.21.3은 JWT `exp`가 지나면 송출 세션을 `connection expired`로 끊는다. 30초 토큰으로 확인했다. 60초로 두면 모든 방송이 1분에 끊긴다. 그래서 TTL이 곧 송출 한 번의 최대 길이다.
- **프론트는 #3 PoC에서 실기기로 확인한 송출 코드를 옮겼다.** 탭 소리와 마이크를 `AudioContext`로 한 트랙에 섞고, H264를 우선한다. 송출 측 `setParameters`로 720p30 / 2.5Mbps를 지킨다. 연결이 `failed`이거나, `disconnected`에서 5초 안에 돌아오지 않으면 송출을 멈추고 알린다. 이때 stop API는 부르지 않는다. 잠깐 끊긴 것으로 방송을 끝내면 안 되고, 그 판단은 #10의 INTERRUPTED가 맡는다.

## Rejected alternatives

- 송출 인증을 지금 `authMethod: http`(MediaMTX가 API에 묻기)로 바꾸기. 이렇게 하면 60초 입장 토큰, `jti` 1회용, 상태 확인을 모두 강제할 수 있다. 하지만 auth/1에서 기각한 "미디어 노드가 API 호출에 묶이는 구조"를 되살린다. #10의 hook과 함께 다시 판단한다.
- 짧은 TTL과 클라이언트 주기 재발급. 토큰이 바뀔 때마다 WHIP 세션을 새로 맺어야 해서 방송이 주기적으로 끊긴다.
- 경로에 시작 회차(epoch)를 넣기. 방송은 READY→LIVE 전이를 한 번만 하므로 회차가 바뀌지 않는다. 종료 후 재송출을 막지 못한다.
- `startedAt`/`endedAt`을 응답에 노출하기. 컬럼만 쓰고 읽지 않는다. 다시보기(#12)나 상태 관리(#10)에서 필요할 때 연다.

## Cautions when touching this

- **종료한 방송에 같은 토큰으로 다시 송출할 수 있다.** 토큰이 만료(12시간)될 때까지 가능하다. 실제 스택에서 stop 뒤 같은 토큰의 WHIP가 201을 받고 경로가 다시 열리는 것을 확인했다. MediaMTX는 서명, 경로, exp만 보고 DB 상태는 보지 않는다. `jti`는 매번 다르게 발급하지만 아무도 검사하지 않는다. #10에서 publish hook이 API에 상태를 묻게 해서 막는다. 그 전까지 auth/1의 "TTL 60초, jti 1회용"과 media/1의 "재연결 시 토큰 재발급 거부"는 지켜지지 않는다.
- LIVE 방송에서 start를 부를 때마다 12시간 토큰이 하나씩 더 생기고 폐기할 수 없다.
- INTERRUPTED에서 start는 지금 409다. #10에서 재연결을 설계할 때 LIVE처럼 재발급할지 정한다.
- 키(`PUBLISH_JWT_PRIVATE_KEY`)를 바꾸면 MediaMTX를 재시작해야 한다. MediaMTX는 JWKS를 처음 인증할 때 받아 1시간 캐시한다(소스로 확인). 지금 JWKS에는 키가 하나뿐이라, 키를 바꾸는 순간 발급된 토큰이 모두 무효가 된다.
- (unverified) 실기기 화면 공유의 2.5Mbps/720p 상한 도달, Android 브라우저.

## Follow-up (2026-11-08, #10)

송출 허가를 MediaMTX `authMethod: http`로 바꿨다. 이제 API가 상태와 1회용 `jti`를 검사한다. 그래서 토큰 TTL은 60초로 돌아갔고, 종료 후 재송출 공백도 닫혔다. 위 "Cautions"의 12시간 토큰과 재송출 항목은 더 이상 맞지 않는다. 자세한 내용은 [media/10-publish-admission-reaper](10-publish-admission-reaper.md).
