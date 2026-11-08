---
issue: 10
area: media
summary: 송출 허가를 MediaMTX http 인증으로 API에 물어 상태·1회용 jti를 강제하고, hook과 advisory lock reaper로 LIVE ⇄ INTERRUPTED(30초) → ENDED를 관리한다
status: confirmed
date: 2026-11-08
paths: [crates/domain/src/broadcast.rs, crates/application/src/media.rs, crates/application/src/reaper.rs, crates/infrastructure/src/persistence/reaper.rs, crates/infrastructure/src/persistence/publish_ticket.rs, crates/api/src/internal.rs, crates/api/src/reaper.rs, deploy/mediamtx/mediamtx.yml]
related: [1, 7]
---

# 방송 상태 관리와 송출 허가

## Background

#7은 MediaMTX `authMethod: jwt`로 송출을 열었다. 이 방식에는 두 가지 문제가 있었다. MediaMTX는 JWT `exp`가 지나면 송출 세션을 끊는다. 그래서 토큰 TTL을 12시간으로 늘려야 했다. 또 MediaMTX는 DB 상태와 `jti`를 보지 않는다. 그래서 종료된 방송에 같은 토큰으로 다시 송출할 수 있었다. 사용자는 이 공백을 #10에서 막기로 정했다. #10은 그 밖에 INTERRUPTED(30초 유예), hook, reaper, 사용자당 동시 방송 1개를 맡는다.

## Decision and reasons

- **송출 허가는 MediaMTX `authMethod: http`로 API가 판단한다. 대상은 publish뿐이다.** 송출을 시도할 때마다 MediaMTX가 `/internal/media/auth`를 호출한다. API는 #7의 EdDSA JWT를 직접 검증하고, 이어서 한 문장으로 `jti`를 소비한다. 이 문장은 소유자, 상태 ACTIVE, 미삭제를 함께 확인하는 `INSERT … SELECT … ON CONFLICT (jti) DO NOTHING`이다. 0행이면 재사용으로 보고 거부한다. 상태 때문에 거부할 때는 `jti`를 소비하지 않는다. 이제 토큰은 입장권일 뿐이라 TTL을 60초로 되돌렸다. 송출 세션은 토큰이 만료돼도 유지된다(65초로 확인).
- **시청에는 callback이 없다.** `read`와 `playback`은 `authHTTPExclude`로 뺐다. callback은 송출 시도와 API 자신의 Control API 호출에서만 생긴다. auth/1은 "미디어 노드가 API에 묶인다"는 이유로 callback 방식을 기각했다. 그 걱정은 요청 수가 많은 시청에 대한 것이었고, 송출 시도 횟수는 그 걱정의 대상이 아니라고 판단했다. 대신 API가 멈추면 새 송출도 거부된다(fail closed). start도 API가 필요하므로 새 의존은 아니다.
- **`/internal/*`은 내부망에 두고 공유 비밀값으로 막는다(HMAC 대신).** MediaMTX의 HTTP 인증과 hook은 요청에 서명할 수 없다. 이미지에는 셸 없이 정적 `wget`만 있다. 그래서 `MEDIA_HOOK_SECRET`을 URL userinfo에 넣어 Basic 인증으로 보내고, API는 상수 시간으로 비교한다. query string 대신 userinfo를 쓴 이유가 있다. Go `net/http`는 오류 로그에서 비밀번호를 `***`로 가린다(API를 멈춘 상태에서 확인). query string이면 API가 응답하지 않을 때 로그에 그대로 남는다. Caddy는 `/api/*`와 `/media/*`만 프록시하므로 `/internal`은 외부에서 닿지 않는다.
- **상태 전이는 `BroadcastEvent` 하나로 관리한다.** 이벤트는 Start, Interrupt, Resume, Stop, Expire 다섯 가지다. 각 이벤트의 출발 상태 집합과 도착 상태를 domain에 두고, SQL은 이를 바인딩한다. 상태 4개 × 이벤트 5개 = 20칸 전체를 테스트로 고정했다. INTERRUPTED에서 start를 부르면 전이 없이 토큰을 다시 발급한다. 유예 안에 다시 연결하려면 토큰이 필요하기 때문이다.
- **hook은 힌트이고, 사실은 조건부 UPDATE가 정한다.** `runOnAvailable`은 Resume, `runOnUnavailable`은 Interrupt로 처리한다. 늦거나 중복된 hook이 ENDED를 되살리지 못한다. hook을 받았는데 그 방송이 활성이 아니면 바로 kick한다.
- **reaper는 5초마다 돈다.** `pg_try_advisory_xact_lock`을 잡은 트랜잭션 하나에서 일하므로 인스턴스가 여럿이어도 한 곳만 실행된다. 순서는 다음과 같다.
  1. 보정(놓친 hook): 송출 중인데 INTERRUPTED면 Resume한다. 송출이 없고 `live_since`가 유예보다 오래된 LIVE면 Interrupt한다.
  2. 활성이 아닌 방송의 송출을 찾는다(종료, 삭제, 없는 id).
  3. 유예가 지난 INTERRUPTED를 Expire한다.
  4. 쓴 티켓을 지운다.
  5. 커밋한 뒤 2와 3에서 찾은 것을 kick한다.

  보정을 만료보다 먼저 하는 이유가 있다. 송출자가 돌아왔는데 hook만 놓친 방송을 끝내지 않기 위해서다. 시각 비교는 DB `now()`(패스 시작 시각)로만 한다.
- **"끝난 방송이 송출 중"을 매 패스마다 다시 kick한다.** 이렇게 하면 두 경우가 함께 해결된다. stop이나 Expire 뒤 kick이 실패해 다시 시도하지 않던 경우, 그리고 허가 직후 stop이 커밋되는 경합이다. 시청은 인증이 없으므로, 이 처리가 없으면 끝난 방송이 계속 공개된다.
- **유령 LIVE 판정 기준은 `live_since`다.** 처음에는 `updated_at`을 썼다. 그러면 제목을 수정할 때마다 판정이 미뤄진다. 그래서 Start와 Resume에서 기록하는 `live_since`로 바꾸고, CHECK `(status = 'LIVE') = (live_since IS NOT NULL)`을 걸었다.
- **사용자당 활성 방송은 하나다.** 부분 UNIQUE 인덱스(LIVE, INTERRUPTED)로 막는다. 위반하면 인덱스 이름으로 `OWNER_ALREADY_LIVE`(409)에 매핑한다. 인덱스 안의 상태 목록이 domain `ACTIVE`와 같은지는 테스트로 확인한다.

## Rejected alternatives

- jwt를 유지하고 hook에서만 막기. `runOnReady`류 hook에는 토큰이 전달되지 않는다. 그래서 `jti`를 검사할 수 없고, 송출이 이미 시작된 뒤에야 끊을 수 있다.
- 짧은 TTL과 주기적 재발급. 토큰이 바뀔 때마다 WHIP 세션이 새로 맺어진다.
- 상태 전이에서는 `version`을 올리지 않기. #6은 "모든 쓰기는 version을 올린다"로 정했고 테스트도 이를 전제로 한다. hook 때문에 편집 폼이 409를 받으면, 폼은 다시 불러오라고 안내한다.
- probe를 트랜잭션 밖에서 먼저 하기. 패스 시작 시각을 `now()`로 고정해 hook과의 경합을 막는 편을 택했다. 대가로 연결 하나가 패스 동안(최대 수 초) idle-in-transaction이 된다.

## Cautions when touching this

- auth/1의 "MediaMTX가 API를 부르지 않는다"는 원칙은 송출에 한해 이 ADR이 대체한다. media/7의 12시간 토큰과 재송출 공백은 해소됐다.
- 비밀값은 MediaMTX 컨테이너 환경변수와 hook 명령 인자에 평문으로 있다. 호스트에 접근할 수 있으면 비밀값도 볼 수 있다. 바꿀 때는 api와 mediamtx를 함께 다시 만든다.
- 다시 연결하는 시점이 유예 끝에 걸리면 Expire와 경합하고, 지면 409다. start는 유예를 연장하지 않는다.
- 쓴 티켓은 만료 후 5분 더 보관한다. API 호스트 시계가 DB보다 느리더라도 재사용을 거부하기 위해서다.
- reaper 종료 대기는 3초가 상한이다. 커밋 뒤 kick 도중 중단되면 다음 패스가 다시 kick한다.
- 마이그레이션 0005는 기존 행을 바꾼다. 소유자별 활성 방송 가운데 id가 가장 큰 것만 남기고 나머지를 종료한다.
- (unverified) 실제 stop과 송출 허가 사이의 경합은 시점을 맞추지 못해 SQL로 흉내 냈다. 이 경우도 hook과 reaper가 kick하는 것은 확인했다.
