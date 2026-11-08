---
issue: 6
area: broadcast
summary: 방송 CRUD를 쓰기 Repository·읽기 Query로 나누고, 소유자·버전·상태 확인을 조건부 UPDATE 한 문장에 넣으며, version은 If-Match로만 받는다
status: confirmed
date: 2026-11-06
paths: [crates/domain/src/broadcast.rs, crates/application/src/broadcast.rs, crates/application/src/ports/broadcast.rs, crates/infrastructure/src/persistence/broadcast.rs, crates/infrastructure/migrations/0003_broadcasts.sql, crates/api/src/broadcast.rs, crates/api/src/openapi.rs]
related: [1, 5, 10]
---

# 방송 CRUD와 인가

## Background

이슈 #1 ADR(`data/1-concurrency-snowflake`)이 조건부 UPDATE, `version` 낙관적 락, soft delete를 정했다. 이번 작업은 방송 등록·조회·수정·삭제와 그 인가를 그 규칙대로 구현하면서 생긴 결정을 남긴다.
범위 밖: 방송 상태 전이(READY→LIVE 등, #10), 시청 화면(#8).

## Decision and reasons

| 결정 | 이유 |
|---|---|
| 쓰기 port `BroadcastRepository`(insert·update·soft_delete)와 읽기 port `BroadcastQuery`(list·get)를 분리 | 읽기는 users JOIN으로 `ownerName`까지 담은 뷰를 돌려주고, 쓰기는 도메인 규칙만 다룬다. 읽기 경로를 캐시·복제본으로 옮길 때 쓰기 쪽이 바뀌지 않는다 |
| 수정은 `WHERE id AND owner_id AND version AND deleted_at IS NULL`, 삭제는 `WHERE id AND owner_id AND status = ANY($3) AND deleted_at IS NULL` 한 문장. 영향 행 0일 때만 `owner_id`를 다시 읽어 NotFound/NotOwner/버전·상태 불일치를 가린다 | 확인과 실행 사이 틈이 없다. 사유 조회는 이미 실패한 쓰기를 설명할 뿐이라 경합이 있어도 권한이 새지 않는다 |
| 삭제 가능 상태는 `BroadcastStatus::DELETABLE` 한 곳. SQL은 그 값을 배열로 바인딩 | 규칙을 SQL 문자열에 다시 적지 않는다 |
| owner는 세션 사용자, version은 `If-Match: "<n>"` 헤더로만 받는다. 요청 본문 DTO에는 `title`, `description`만 있다 | 프로젝트 규칙(요청 DTO에서 owner_id/status/version 바인딩 금지). ETag/If-Match는 HTTP 표준의 낙관적 락 표현이다 |
| `If-Match`는 강한 태그만 허용(`W/`와 `*`는 400) | RFC 9110은 If-Match에 강한 비교를 요구한다. 서버는 강한 ETag만 발급한다 |
| 목록은 키셋 페이지네이션(`id < before`, 최신순, `limit+1`로 다음 페이지 판단)과 `Cache-Control: public, max-age=5` | Snowflake id가 시간순이라 OFFSET 없이 일정한 비용이다. 공개 캐시가 안전하려면 목록에 사용자별 필드와 `Set-Cookie`가 없어야 하고, 이를 테스트로 고정했다 |
| 제목·설명 길이를 DB CHECK로도 강제(제목은 Rust `trim()`과 같은 공백 집합으로 `btrim`) | 읽기 경로가 저장된 행을 도메인 규칙으로 다시 파싱하므로, 규칙 밖 행 하나가 공개 목록 전체를 500으로 만들었다(리뷰에서 재현). DB와 도메인이 같은 집합을 받는지 테스트가 확인한다 |
| OpenAPI는 utoipa로 생성해 `web/src/api/openapi.json`에 스냅샷으로 커밋. 다르면 `cargo test`가 실패, `UPDATE_OPENAPI=1`로 갱신. 웹은 openapi-typescript로 타입 생성, CI가 최신인지 검사 | 서버 계약과 프론트 타입이 어긋나면 CI에서 바로 드러난다 |
| 타입 생성기는 `web/tools/gen-api` npm workspace에 TypeScript 5와 함께 둔다 | openapi-typescript가 TS 5 컴파일러 API에 의존하는데 앱은 API가 없는 TS 7을 쓴다 |
| 서버 에러 메시지는 영어로 두고 웹이 코드→한국어 표로 보여 준다 | `code`가 계약이고 메시지는 사람용이다. 모르는 코드는 서버 메시지로 대체한다 |

## Rejected alternatives

- testcontainers: CI에 Postgres 서비스가 이미 있고 `#[sqlx::test]`가 테스트마다 새 DB를 만든다. 의존성과 Docker-in-CI를 늘리지 않는다.
- version을 요청 본문으로 받기: 프로젝트 규칙 위반.
- 영향 행 0일 때 판정을 CTE 한 문장으로: 사유 조회는 실패 경로에서만 돌아 비용이 작고, 두 문장이 읽기 쉽다.
- 소유자 이름 전용 port 메서드: 쓰기마다 PK 조회 1번이고 쓰기는 드물다.
- 서버 메시지 한국어화: API 계약을 언어에 묶는다.

## Cautions when touching this

- LIVE·INTERRUPTED 방송도 제목·설명은 수정할 수 있다(방송 중 제목 변경). 삭제만 막는다.
- 목록 핸들러에 세션 extractor를 붙이거나 사용자별 필드를 넣으면 공유 캐시가 다른 사용자에게 응답을 내준다. 그때는 `Cache-Control`을 `private`로 바꾼다.
- 캐시된 목록 응답에는 처음 요청한 사람의 `x-trace-id`가 남는다. 장애 문의 때 trace id가 맞지 않을 수 있다.
- 길이 한도는 domain 상수, DB CHECK, 웹 한국어 메시지 세 곳에 있다. 한도를 바꾸면 마이그레이션과 웹 메시지도 바꾼다(DB 쪽은 테스트가 잡고, 웹 메시지는 잡지 못한다).
- 수정·삭제 모두 `version`을 올린다. 동시 수정·삭제 경합 테스트가 이를 전제로 한다.
