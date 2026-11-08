---
issue: 27
area: common
summary: 요청 10초·본문 64KB 상한을 라우터에 걸고, 만료 세션 정리와 데모 방송 초기화를 reaper 패스에 넣되 정리 실패가 상태 전이를 막지 않게 한다
status: confirmed
date: 2026-11-08
paths: [crates/api/src/lib.rs, crates/api/src/error.rs, crates/application/src/reaper.rs, crates/infrastructure/src/persistence/reaper.rs, crates/infrastructure/src/persistence/session.rs]
related: [1, 4, 5, 10, 11]
---

# 요청 상한과 reaper 정리 작업

## Background

common/1, infra/1, auth/1, auth/5가 요청 타임아웃, 본문 64KB, 만료 세션 전역 정리, 데모 계정 매일 초기화를 정했지만 main에 들어가지 않았다(ADR 상세판을 코드와 대조하다 발견). 범위 밖: 보안 헤더 전반(#14), hyper 수준의 헤더 읽기 타임아웃(slowloris).

## Decision and reasons

| 결정 | 이유 |
|---|---|
| 라우터를 `bounded`와 `long_lived`로 나누고 `bounded`에만 10초 `time_limit` 미들웨어. 지금 `long_lived`는 비어 있고 #11 SSE가 들어간다 | 상한은 기본으로 걸리고(deny by default), 오래 열리는 경로만 명시적으로 빠진다. WHIP은 API가 아니라 MediaMTX로 가므로 예외 대상이 아니다 |
| tower-http `TimeoutLayer` 대신 `tokio::time::timeout`을 감싼 미들웨어 | `TimeoutLayer`는 빈 본문을 돌려줘 `{code, message, traceId}` 봉투가 깨진다 |
| 시간 초과는 503 `REQUEST_TIMEOUT` | 408은 브라우저가 스스로 재시도할 수 있어 POST가 두 번 나갈 수 있다 |
| 본문은 `DefaultBodyLimit` 64KB. Json·Bytes 추출 거부를 `ApiError::rejected_body`로 받아 413은 `PAYLOAD_TOO_LARGE`, 나머지는 `INVALID_REQUEST` | `Bytes`를 그대로 받으면 axum이 봉투 없는 413을 직접 보낸다. `From<JsonRejection>`은 `DomainError` blanket impl과 coherence 충돌 우려가 있어 명시 함수로 둔다 |
| 만료 세션 정리와 데모 초기화를 기존 reaper 패스(advisory lock 트랜잭션)의 단계로 넣는다 | 단일 실행 보장과 DB `now()` 기준을 이미 가진 곳이다. 새 주기 작업과 새 lock 키를 만들지 않는다 |
| 로그인 시 사용자별 만료 세션 삭제를 지운다 | 만료 판정 SQL을 reaper 한 곳에만 둔다. `resolve`가 만료를 직접 판정하므로 로그인 동작은 같다 |
| 데모 초기화 = 데모 계정의 READY/ENDED 방송 중 KST 오늘 0시 이전에 만든 것을 패스마다 soft delete | 스케줄 상태 없이 수렴한다(멱등). 패스가 빠지거나 겹쳐도 결과가 같다. 삭제는 soft, READY/ENDED에서만이라는 규칙을 지킨다. 활성 방송은 끝난 뒤 초기화된다 |
| 정리 단계(spent ticket, 세션, 데모)는 각자 SAVEPOINT에서 돌고 실패는 `Reaped`에 기록만 한다 (리뷰 F1) | 정리 실패가 같은 트랜잭션의 `reconcile`/`expire`를 롤백시키면 방송 상태 머신이 멈춘다 |

## Rejected alternatives

- 데모 초기화를 cron식 "자정 1회" 작업으로: 마지막 실행 시각을 저장해야 하고, 그 시각에 인스턴스가 없으면 하루를 건너뛴다.
- 데모 방송 hard delete: 저장소 규칙(soft delete)과 어긋나고 `publish_token_uses` FK를 먼저 지워야 한다. 데모 규모에서 행 증가는 무시할 만하다.
- 데모 세션도 매일 폐기: 사용 중인 심사자를 로그아웃시킨다. 초기화 대상은 그가 만든 데이터다.
- 세션 정리용 `expires_at` 인덱스 (리뷰 F2): OR 조건의 `last_seen_at` 쪽 때문에 스캔이 남고, `last_seen_at` 인덱스는 슬라이드 UPDATE의 HOT을 잃는다. 세션 10만 건에서 DELETE가 6~14ms로 측정됐다(5초 주기).

## Cautions when touching this

- SSE 같은 스트리밍 라우트를 `bounded` 쪽에 붙이면 10초마다 끊긴다. `app()`의 `long_lived` 자리에 붙인다.
- 타임아웃은 핸들러 future를 drop한다. 트랜잭션은 롤백되고, 커밋 후 보내지 못한 kick은 reaper가 다시 보낸다. 커밋 전후로 여러 쓰기를 나눠 하는 핸들러를 새로 만들면 중간 상태가 남을 수 있다.
- OpenAPI 설명의 64 KiB, 10 seconds는 `BODY_LIMIT`, `REQUEST_TIMEOUT`과 테스트로 묶여 있다.
- 데모 하루의 경계는 `Asia/Seoul`이다. Postgres 이미지에 tzdata가 있어야 한다(현재 compose 이미지에서 확인).
- (unverified) 세션 수가 수백만이 되면 5초마다 전체 스캔 비용을 다시 잰다.
