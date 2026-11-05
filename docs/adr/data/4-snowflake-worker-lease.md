---
issue: 4
area: data
summary: Snowflake 발급을 domain 순수 상태 머신과 PG worker lease로 나누고, 에러 응답 봉투는 미들웨어가 traceId와 함께 쓴다
status: in-progress
date: 2026-11-06
paths: [crates/domain/src/id.rs, crates/infrastructure/src/id.rs, crates/infrastructure/src/persistence/worker_lease.rs, crates/api/src/error.rs, crates/api/src/config.rs]
related: [1]
---

# Snowflake 발급 구조, worker lease, 에러 응답 봉투

## Background

이슈 #1 ADR(`data/1-concurrency-snowflake`)이 Snowflake(41/10/12, epoch 2026-01-01)와 PG lease 기반 worker id를 정했다. 이번 작업은 그 결정을 코드로 옮기면서 생긴 구조 결정을 남긴다. 공통 에러 응답, `Secret<T>`, 환경변수 설정도 같은 기반 작업에 들어 있다.
범위 밖: ID를 실제로 쓰는 handler, extractor rejection의 봉투화, 시계 주입 가능한 테스트 구조.

## Decision and reasons

| 결정 | 이유 |
|---|---|
| 발급 규칙은 domain의 `SnowflakeSequencer` 상태 머신. `next(now_ms)`가 발급, `ClockBehind`, `SequenceExhausted`, `TimestampOverflow` 중 하나를 돌려주고 실패 시 상태를 바꾸지 않는다 | 시계를 인자로 받으니 시계 역행·시퀀스 소진 테스트를 가짜 시계 없이 쓸 수 있다. domain은 std만 쓴다 |
| 시퀀서를 `last_ms = floor, seq = 4095`로 초기화한다 | "재시작 시 `now > last_ms`까지 대기"가 별도 분기 없이 같은 ms 소진으로 표현된다 |
| 시계는 기동 시 벽시계 1회 + `Instant` 경과 | 프로세스 안에서는 역행이 일어날 수 없다. 역행은 재시작 경계에서만 생기고 lease의 `last_ms`로 막는다 |
| lease 만료 판정은 DB `now()`만 쓴다. 획득은 `FOR UPDATE SKIP LOCKED` 서브쿼리 + 바깥 `expires_at <= now()` 재확인의 조건부 UPDATE, 갱신·반납은 `holder` 조건부 UPDATE, `last_ms`는 `GREATEST`로만 올린다 | 인스턴스마다 시계가 다를 수 있다. 프로젝트 규칙(조건부 UPDATE, 영향 행 0이면 도메인 에러)과 같은 모양이다 |
| 1024개 슬롯을 마이그레이션에서 미리 채운다 | 획득이 INSERT 경쟁 없이 UPDATE 한 문장이 된다 |
| 생성기는 `sent_at + 2/3·TTL`까지만 발급하고, 넘으면 `LeaseExpired`. heartbeat는 TTL/3 주기 | DB가 슬롯을 다른 인스턴스에 넘기기 전에 이쪽이 먼저 멈춘다 |
| 종료 시 발급을 막고(`valid_until = 0`) 시퀀서 lock으로 진행 중 발급을 기다린 뒤 실제 `last_ms`로 반납 | 반납된 슬롯으로 ID가 나가지 않고, 다음 holder의 floor가 정확하다 |
| `IdGenerator::next_id`는 동기 함수 | 기다림은 1ms 미만 spin뿐이라 await 지점이 필요 없다 |
| 에러는 domain의 `DomainError` trait(`kind`, `code`)로 분류하고, api가 `ErrorKind`를 HTTP status로 바꾼다 | 기능별 에러 enum이 늘어도 매핑은 한 곳이다 |
| `ApiError`는 status와 `ErrorPayload` extension만 만들고, `trace_id` 미들웨어가 JSON 본문 `{code,message,traceId}`와 `x-trace-id` 헤더를 쓴다 | traceId를 handler마다 넘기지 않는다. 404 fallback도 같은 경로를 탄다 |
| `Secret<T>`는 domain에 직접 구현(`Debug`만 가림, `Display`·serde 없음) | 20줄 남짓이고, domain 의존성 허용 목록(thiserror)을 넓히지 않는다 |
| sqlx는 `infrastructure`에서만. DB 연결도 `infrastructure::persistence::connect`가 한다 | `deny.toml` ban이 sqlx를 infrastructure 밖에서 금지한다 |
| sqlx는 TLS 없이 쓴다 | Postgres는 compose 내부망에만 있고 외부 포트가 없다 |
| PG 통합 테스트는 `#[ignore]` + CI postgres 서비스에서 `--include-ignored` | DB 없는 로컬 `cargo test`는 그대로 통과하고, CI에서는 실제로 돈다. DB가 없을 때 조용히 통과하는 테스트는 만들지 않는다 |

## Rejected alternatives

- 생성기 안에서 시계를 직접 읽는 구조: 시계 역행 테스트에 가짜 시계 주입이 필요하다.
- 앱 시계로 lease 만료 판정: 인스턴스 간 시계 오차가 곧 중복 worker id가 된다.
- `secrecy` crate: (unverified) 최신 버전에 `Secret<T>` 타입이 없고, domain 의존성이 늘어난다.
- handler가 traceId를 받아 에러 본문을 직접 만드는 방식: 모든 handler에 같은 인자가 퍼진다.
- 첫 번째 빈 슬롯을 worker_id 순으로 고르기: 방금 반납·만료된 슬롯이 바로 재사용된다. `ORDER BY expires_at`로 가장 오래 비어 있던 슬롯부터 쓴다.

## Cautions when touching this

- heartbeat가 DB에 기록하는 `last_ms`는 갱신 시점의 값이다. 프로세스가 비정상 종료하면 그 뒤 최대 2/3·TTL 동안 발급한 ms를 DB가 모른다. 지금은 "같은 슬롯을 다시 받은 인스턴스의 시계가 TTL/3 이상 뒤처지지 않는다"는 가정에 기대고 있다. 이슈 #1 ADR의 "재시작 시 `now > last_ms`까지 대기"는 정상 종료 경로에서만 성립한다.
- lease를 한 번 `Lost`로 잃으면 heartbeat task가 끝나고 발급이 멈춘 채로 남는다. healthz는 이 상태를 보지 않는다.
- TTL 최소값(3초)은 heartbeat 주기 TTL/3과 발급 여유 TTL/3을 1초 이상으로 두려는 값이다. TTL을 바꾸면 두 값이 함께 움직인다.
- 시퀀스 소진 시 std `Mutex`를 쥔 채 1ms 미만 spin한다. 발급 경합이 커지면 tokio worker 스레드가 막힌다.
- 응답 봉투는 `ErrorPayload` extension이 붙은 응답만 바꾼다. axum이 직접 만드는 응답(405, extractor rejection, panic)은 봉투를 거치지 않는다.
- Snowflake ID를 JSON에 넣을 때는 반드시 `api::json::snowflake_id`를 쓴다. 숫자로 나가면 JS에서 2^53 이상이 조용히 깨진다.
