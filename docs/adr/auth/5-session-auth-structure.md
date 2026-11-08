---
issue: 5
area: auth
summary: 세션 인증을 Credential enum·SessionStore port·CSRF 미들웨어·인스턴스별 속도 제한으로 구현하고, 토큰 생성·해시를 adapter에 둔다
status: confirmed
date: 2026-11-06
paths: [crates/domain/src/auth.rs, crates/application/src/auth.rs, crates/application/src/ports/auth.rs, crates/infrastructure/src/auth.rs, crates/infrastructure/src/persistence/session.rs, crates/api/src/csrf.rs, crates/api/src/rate_limit.rs]
related: [1, 6, 10, 14]
---

# 세션 인증 구현 구조

## Background

이슈 #1 ADR(`auth/1-hybrid-session-jwt`)이 웹은 DB 세션 쿠키, 비밀번호는 argon2id, 계정 열거 방지, 데모 계정을 정했다. 이번 작업은 그 결정을 hexagonal 계층에 옮기면서 생긴 구조 결정을 남긴다.
범위 밖: 송출 JWT(#7), 데모 계정 매일 초기화(초기화할 데이터가 아직 없다), 만료 세션 전역 정리(#10 reaper), 보안 헤더 전반(#14).

## Decision and reasons

| 결정 | 이유 |
|---|---|
| 계정 자격은 `Credential::{Password(hash), DemoOnly}` enum. DB는 `CHECK (is_demo <> (password_hash IS NOT NULL))` | `is_demo` bool과 `Option<hash>`가 따로 놀면 "비밀번호 있는 데모 계정" 같은 상태가 생긴다. 행 하나가 정확히 한 `Credential`로 매핑된다 |
| 토큰 생성(OS RNG 256bit, base64url)과 SHA-256 해시는 `SessionStore` adapter 안에 둔다. port는 `create(user) -> token`, `resolve(token) -> user`, `revoke(token)` | application의 의존 allowlist(domain, thiserror, async-trait)에 crypto crate를 넣지 않는다. Redis로 바꿀 때도 port가 그대로다 |
| 만료(유휴 30분, 절대 7일)는 DB `now()`로만 판정. `resolve`는 한 문장(CTE)으로 확인하고 `last_seen_at`은 60초 넘게 지났을 때만 갱신 | 인스턴스 시계가 달라도 판정이 같다. 매 요청 UPDATE의 쓰기 증폭을 1분 1회로 줄인다. 대가로 유휴 만료가 최대 60초 일찍 올 수 있다 |
| 로그인 실패는 계정 없음·틀린 비밀번호·데모 계정 모두 `verify`를 정확히 한 번 호출하고 `INVALID_CREDENTIALS` 하나로 답한다. `verify(password, None)`이 adapter의 더미 해시를 검증한다 | 응답 내용과 시간으로 계정 존재를 구분할 수 없다. 이 규칙은 application 단위 테스트(호출 횟수)로 고정했다 |
| 가입·로그인·데모 로그인 모두 요청에 딸려 온 이전 세션을 폐기하고 새 토큰을 발급 | 세션 고정 방지. 가입도 곧 로그인이라 같은 규칙을 쓴다 |
| argon2 작업은 `spawn_blocking` + 세마포어(코어 수). permit은 소유형으로 blocking 작업 안으로 옮긴다 | 요청이 취소돼도 blocking 작업은 끝까지 돈다. permit이 async 쪽에 있으면 취소 시 풀려 동시 실행 상한이 깨진다(리뷰에서 재현 후 수정) |
| argon2 파라미터는 이름 붙은 상수 하나를 더미와 실제 해시가 공유 | 파라미터를 바꾸면 더미와 기존 해시의 비용이 달라져 계정 존재가 드러난다. 바꿀 때는 rehash 정책을 같이 정한다 |
| 유출 목록은 SecLists 100만 건 목록에서 10~128자만 남겨 바이너리에 내장(대소문자 무시) | 10자 미만은 길이 규칙이 이미 거부한다. 원래 계획(상위 1만 개)은 10자 이상이 51개뿐이라 실효성이 없었다 |
| CSRF는 `/api` 전체에 미들웨어로 건다. POST·PUT·PATCH·DELETE는 허용 `Origin` 정확히 일치 + `X-Requested-With: live-platform` | 이후 이슈의 라우트가 자동으로 보호된다(deny by default). 그래서 허용되지 않은 메서드도 헤더가 없으면 405 대신 403이다 |
| `ALLOWED_ORIGINS`는 브라우저가 보내는 형태만 허용(기본 포트·앞자리 0·부호 거부) | 잘못 쓴 값으로 기동은 되고 모든 쓰기가 403이 되는 장애를 기동 시점 오류로 바꾼다 |
| 속도 제한은 인스턴스 메모리의 governor(GCRA). 키는 Caddy가 쓰는 XFF 마지막 항목, 없으면 peer. IPv6는 /64로 묶고 IPv4-mapped는 IPv4로 본다 | 제한은 방어선이지 진실의 원천이 아니라 메모리로 충분하다. api 포트는 외부에 열리지 않고 Caddy는 들어온 XFF를 신뢰하지 않는다 |
| 데모 계정은 `DEMO_LOGIN_ENABLED`일 때만 기동 시 seed. `demo-` 접두사는 가입에서 예약. 같은 이름이 비밀번호 계정이면 seed가 실패해 기동을 멈춘다 | 데모 로그인이 일반 계정으로 들어가는 경로를 구조적으로 막는다 |

## Rejected alternatives

- 토큰 생성을 application 유스케이스에서: crypto crate가 안쪽 계층으로 들어온다.
- 만료 세션 전역 purge 작업을 이번에: advisory lock 기반 주기 작업은 #10 reaper 인프라와 같이 만든다. 지금은 로그인 시 그 사용자의 만료 세션만 지운다.
- 속도 제한을 Redis나 DB로: 인스턴스가 하나이고, 제한은 근사치여도 된다.
- 로그인에 별도 길이 규칙(상한만): 지금은 더 짧은 비밀번호를 가진 계정이 없다.
- `PortError`에 원인 에러 체인 보존, 평문 비밀번호 zeroize: 원인은 로그로만 쓰이고, 메모리를 읽을 수 있는 공격자는 이미 위협 모델 밖이다.

## Cautions when touching this

- 비밀번호 최소 길이를 올리면 로그인도 같은 `Password::parse`를 쓰므로 기존 사용자가 로그인하지 못한다. 그때는 로그인용 파싱을 분리한다.
- GCRA라 "버스트 N, 이후 평균 N/기간"이다. 첫 구간에는 표기 수치보다 더 통과한다(가입 첫 10분 최대 9회).
- 사용자명당 로그인 제한(5/분)은 남이 일부러 틀려 특정 계정을 잠시 잠글 수 있다. 이슈가 요구한 제한이라 받아들인 trade-off다.
- 배포 호스트가 IPv6를 받는데 Docker 브리지가 IPv4 전용이면 모든 IPv6 클라이언트가 게이트웨이 IP 한 키로 묶일 수 있다. (unverified) 운영 호스트 설정은 확인하지 않았다.
- 세션 쿠키는 `__Host-`라 HTTPS(또는 localhost)에서만 동작한다. 로컬 Vite 개발은 `ALLOWED_ORIGINS`에 Vite origin을 넣어야 한다(`web/README.md`).
- 유출 목록 파일은 비밀번호 문자열 11만여 개다. gitleaks는 현재 이를 탐지하지 않았지만 규칙이 바뀌면 allowlist가 필요할 수 있다.

## Follow-up (2026-11-08, 코드 대조)

"만료 세션 전역 정리는 #10 reaper와 같이 만든다"고 했지만 #10(PR #24)의 reaper는 방송과 송출 토큰만 다룬다. 지금도 로그인 때 그 사용자의 만료 세션만 지운다. #27에서 넣는다.
