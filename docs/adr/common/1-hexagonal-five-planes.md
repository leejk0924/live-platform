---
issue: 1
area: common
summary: Hexagonal 4-crate workspace와 5개 plane 분리로 인프라 미정 상태에서 확장성을 확보
status: confirmed
date: 2026-11-05
paths: [crates/domain, crates/application, crates/infrastructure, crates/api]
related: []
---

# Hexagonal 구조와 5개 plane

## Background

인프라와 프레젠테이션이 확정되지 않았다. 7일 안에 필수 기능을 완성해야 하지만, 시청자 500만 규모까지 확장 가능한 방향성이 필요하다. 사용자는 SOLID를 중시하는 Java 개발자이며 Rust를 학습 중이다.

## Decision and reasons

- **Cargo workspace 4개 crate**: `domain` / `application` / `infrastructure` / `api`. 의존 방향 위반이 컴파일 에러가 되어 DIP를 컴파일러가 강제한다.
- **DI는 `Arc<dyn Trait>`**: Java의 interface + 생성자 주입과 같은 감각. 제네릭 정적 디스패치보다 학습 비용이 낮고 adapter 교체가 쉽다.
- **port는 작게(ISP)**: 하나의 adapter가 여러 trait를 구현하고, 유스케이스는 필요한 trait만 받는다.
- **HTTP도 inbound adapter**: presentation이 바뀌어도 adapter 추가로 대응.
- **5개 plane**: Control / Ingest / Delivery / Realtime / Data. 각 plane이 늘어나는 기준이 다르므로(요청 수, 송출자 수, 시청자 수 등) 경계를 나눠야 독립 확장할 수 있다. 지금은 1대에 모두 올리되 compose 서비스와 코드 경계는 plane 단위.
- **CQRS-lite**: 쓰기 Repository와 읽기 `BroadcastQuery`를 분리. 읽기는 나중에 replica/캐시로 교체.
- **OpenAPI(utoipa) 계약**: 프론트 타입 클라이언트를 생성. presentation 경계가 계약으로 고정된다.

## Rejected alternatives

- 단일 crate + 모듈 분리: 의존 방향을 리뷰로만 지켜야 한다.
- Leptos(Rust 풀스택 WASM): WebRTC 생태계가 미성숙, 7일 리스크.
- Next.js: SSR을 위한 Node 서버 추가, 이 서비스에서 이득 적음.
- SeaORM/Diesel: sqlx가 조건부 UPDATE·`ON CONFLICT`·advisory lock을 SQL 그대로 드러내 리뷰가 쉽다.

## Cautions when touching this

- `domain`·`application`에 AWS, MediaMTX, Postgres, axum 이름이 들어가면 경계가 깨진 것이다.
- async trait을 `dyn`으로 쓰려면 `#[async_trait]`이 필요하다.
- 요청 DTO와 도메인 모델을 섞지 않는다(mass assignment 방지).

## Follow-up (2026-11-06): 의존 방향 강제 방식

- **안쪽 crate는 허용 목록, 바깥쪽은 금지 목록**으로 나눈다.
  - `scripts/check-layer-deps.sh`가 `domain`·`application`의 직접 의존을 허용 목록과 비교하고, 목록 밖이면 CI를 실패시킨다(deny by default).
  - 허용 목록: `domain` = `thiserror`, `application` = `domain`, `thiserror`, `async-trait`.
  - `deny.toml`의 `wrappers`는 "axum·tower-http는 api만, sqlx·aws·reqwest·jsonwebtoken은 infrastructure만"처럼 adapter 쪽 경계만 담당한다.
- 이유: 코드 리뷰(jk:code-review 2회)에서 같은 지적이 반복됐다. 금지 목록만으로는 목록에 없는 crate(`reqwest`, `sqlx-core`)가 빠져나가고, `hyper` 규칙은 계획된 S3·Toss adapter를 막았다.
- 시간 타입 crate(`time`/`chrono`)는 `domain`에 넣지 않는다. 필요해지면 #6에서 `i64` 밀리초 값 객체와 비교해 다시 결정한다.
- `serde`는 `domain`에 넣지 않는다. 직렬화는 `api` DTO의 책임이다(mass assignment 방지).
- 요청 타임아웃은 #4, shutdown drain 상한은 #11(SSE)에서 넣는다.

## Follow-up (2026-11-08, 코드 대조)

"요청 타임아웃은 #4에서 넣는다"고 했지만 #4(PR #20)에 들어가지 않았고 main에도 없다. #27에서 넣는다.
