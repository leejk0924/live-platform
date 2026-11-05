---
issue: 1
area: comment
summary: 댓글은 REST로 쓰고 SSE로 push하며, 샘플링 없이 전부 저장하고 쓰기 측 제어로 대규모 채팅을 다룬다
status: confirmed
date: 2026-11-05
paths: [crates/application/src/ports/event_bus.rs, crates/infrastructure/src/realtime, crates/api/src/routes/comments.rs]
related: [11]
---

# 댓글 실시간 전달과 대규모 채팅 정책

## Background

요구사항은 방송별 댓글 CRUD와 작성자 검증이다. 설계 방향은 피크 500만 동시 시청이므로, 실시간 전달과 저장이 인스턴스 수평 확장에 열려 있어야 한다. 구현은 #11.

## Decision and reasons

- **쓰기는 REST, 읽기 push는 SSE**. 실시간 채널이 단방향이면 충분하다.
  - 인증 미들웨어와 작성자 검증(원자적 SQL)을 REST 그대로 재사용한다.
  - 브라우저 `EventSource`가 자동 재연결하고 `Last-Event-ID`를 보내므로, 서버는 그 이후 댓글만 다시 보내면 된다.
  - 일반 HTTP라 Caddy, ALB, HTTP/2 멀티플렉싱을 그대로 통과한다. axum `Sse<impl Stream>`으로 구현한다(Java의 `SseEmitter`와 같은 감각).
- **`EventBus` port**: 지금은 PG NOTIFY adapter, 확장 시 Redis/NATS. SSE 게이트웨이가 N대가 되어도 같은 이벤트를 받는다.
- **200ms 배치 전송**: 연결 수만큼 생기는 write 시스템콜을 줄인다.
- **샘플링하지 않는다. 모든 댓글을 저장한다.** 사용자가 "일부 댓글만 전달"을 받아들이지 않았다. 목록 API로 언제든 전부 조회할 수 있다.
- **대신 쓰기 쪽에서 줄인다.** 사람은 초당 5~10개 이상 읽지 못한다.
  - 사용자당 초당 1개(`RateLimiter`), 방송자가 켜는 슬로우 모드(`ChatPolicy`).
  - 방송자와 도네이션 메시지는 우선 전달 레인으로 항상 즉시 push.
- **저장 확장 경로**: 단건 INSERT → 묶음 INSERT/COPY → 로그 우선(Kafka/NATS) → 시간 파티셔닝 → ScyllaDB. Snowflake ID가 DB 없이 생성되고 `CommentRepository` port 뒤에 있어 유스케이스는 바뀌지 않는다.

## Rejected alternatives

- Polling: 500만 명이 2초마다 요청하면 250만 rps. 새 댓글이 없어도 비용이 든다.
- Long polling: SSE 대비 이점 없이 재연결마다 요청이 새로 생긴다.
- WebSocket: 양방향이 필요 없는데 인증, 재연결, 놓친 메시지 복구, heartbeat를 직접 만들어야 한다. 양방향 채팅이 필요해지면 transport adapter로 추가한다.
- WebTransport: 브라우저 지원과 생태계가 미성숙.
- 시청자 10만 이상 방송의 댓글 샘플링: 처음에 제안했으나 사용자가 거절했다.

## Cautions when touching this

- `EventSource`는 커스텀 헤더를 못 붙인다. SSE 인증은 쿠키에 의존하고, SSE 엔드포인트는 읽기 전용으로 유지한다.
- HTTP/1.1에서는 도메인당 연결 6개 제한이 있다. 프록시가 HTTP/2를 쓰는지 유지해야 한다.
- 지금 `RateLimiter`는 인스턴스 메모리라서, 인스턴스가 늘면 근사치다. 정확한 제한은 Redis adapter가 필요하다.
- (unverified) PG NOTIFY payload는 기본 8000바이트 미만으로 제한된다. 본문 대신 ID와 최소 필드만 보내는 편이 안전하다.
- SSE 연결 수만큼 파일 디스크립터를 쓴다. api 컨테이너 `nofile` 제한과 IP당 연결 20개 제한을 함께 유지한다.
