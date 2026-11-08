---
issue: 11
area: comment
summary: 댓글 변경마다 방송별 advisory lock 아래 revision을 받고 같은 트랜잭션에서 pg_notify로 알려, SSE는 revision 커서로 Last-Event-ID 복구와 200ms 배치를 한다
status: confirmed
date: 2026-11-09
paths: [crates/domain/src/comment.rs, crates/application/src/comment.rs, crates/application/src/ports/comment.rs, crates/application/src/ports/event_bus.rs, crates/application/src/ports/rate_limit.rs, crates/infrastructure/src/persistence/comment.rs, crates/infrastructure/src/realtime.rs, crates/infrastructure/src/rate_limit.rs, crates/infrastructure/migrations/0007_comments.sql, crates/api/src/comment.rs, crates/api/src/comment_stream.rs, web/src/comments]
related: [1, 13, 14, 27]
---

# 댓글 변경 피드: revision 커서, 트랜잭션 안 NOTIFY, SSE 배치

## Background

comment/1이 "쓰기는 REST, push는 SSE, `EventBus` port(PG NOTIFY), 200ms 배치, 쓰기 측 속도 제한"을 정했다. #11은 이를 구현하면서 SSE가 끊겼다 다시 붙을 때 무엇을 다시 보낼지(`Last-Event-ID`), 여러 API 인스턴스에서 이벤트 순서를 어떻게 지킬지를 정해야 했다.

## Decision and reasons

| 결정 | 이유 |
|---|---|
| 댓글의 모든 쓰기(작성, 수정, 삭제)가 전역 시퀀스에서 `revision`을 새로 받는다. 삭제는 soft delete이고 tombstone(`body: null`)으로 피드에 남는다 | "revision > R인 변경"이 인덱스 한 범위(`(broadcast_id, revision)`)라 재생과 따라잡기가 쿼리 하나다. 이벤트 로그 테이블이 따로 필요 없다 |
| 쓰기마다 트랜잭션에서 `pg_advisory_xact_lock(0x636d6e74, 방송 id 하위 32비트)`을 먼저 잡는다 (리뷰 F1) | `nextval`은 쓰기 시점, 커밋은 나중이라 낮은 revision이 늦게 커밋되면 `revision > R` 복구에서 영구히 빠졌다. 방송별로 쓰기를 줄 세워 커밋 순서와 revision 순서를 같게 한다. 두 키 advisory lock은 한 키 lock(reaper)과 키 공간이 다르다 |
| 댓글 변경 알림은 같은 트랜잭션 안에서 커밋 직전에 `pg_notify`로 보낸다. 알림이 실패하면 쓰기도 실패한다 (리뷰 F24) | PostgreSQL은 알림을 커밋할 때 커밋 순서대로 전달한다. lock과 합쳐 라이브 전달도 revision 순서가 되어, 커서 이하의 이벤트는 버려도 된다. 커밋 뒤 따로 발행하면 순서가 뒤집히거나 다른 인스턴스가 조용히 놓쳤다 |
| `EventBus::publish`는 저장 쓰기가 아닌 이벤트(채팅 정책, #13 도네이션의 비저장 알림)에만 쓴다. best effort이고 실패하면 이 인스턴스의 팔로워에게 `Missed`를 보낸다 | 정책은 매번 저장소에서 다시 읽으므로 놓쳐도 다음 따라잡기에서 맞는다 |
| NOTIFY payload에 댓글 전체를 담는다. 프로세스마다 `PgListener` 하나가 받아 방송별 `tokio::sync::broadcast`(1024)로 나눠 준다 | 본문 500자, 제어 문자 금지라 최악의 JSON도 8000바이트 한도 안이다(`the_largest_comments_fit_a_notification`이 실제 `pg_notify`로 확인). 팔로워 수만큼 DB를 읽지 않는다 |
| 구독자가 밀리거나(lag) listener가 다시 LISTEN을 건 뒤에는 `Delivery::Missed`를 보내고, 피드가 커서부터 `changes_since`로 따라잡는다. 501건 이상이면 `reset`(목록 다시 읽기) | 전달은 best effort이고 진실은 저장소에 있다. `Missed`는 LISTEN 복구가 끝난 뒤에만 보낸다(리뷰 F2). 그 전에 보내면 복구 사이의 알림을 다시 잃는다 |
| 커서의 시작은 revision `0`이다. 목록 응답은 항상 `revision`(빈 방송이면 `"0"`)을 주고, 클라이언트는 그것을 `?after=`로 넘긴다. `Last-Event-ID`가 있으면 그쪽이 이긴다 (리뷰 F7) | 빈 목록에서 `null`이면 목록과 스트림 사이의 첫 댓글을 놓쳤다. `EventSource`는 첫 연결에 헤더를 못 붙이고 재연결에는 스스로 붙인다 |
| SSE는 변경을 200ms 모아 `comments` 이벤트 하나로 보내고 `id`는 그 묶음의 최고 revision이다. 방송자 댓글(priority), `policy`, `reset`은 즉시 보낸다. 열 때와 따라잡은 뒤에는 현재 `policy`를 한 번 보낸다(리뷰 F10) | 연결 수만큼의 write를 줄인다. 방송자 우선 레인(comment/1)은 기다리지 않는다 |
| SSE 라우트는 `long_lived` 라우터, 클라이언트(`ClientKey`)당 20개, 종료 신호에서 `close_all`로 모든 스트림을 끝낸다 | 10초 상한 밖이어야 하고(common/27), graceful shutdown이 끝나지 않는 연결을 기다리지 않게 한다. compose에서 열린 스트림이 있어도 api 종료 0.56초 |
| 속도 제한은 `RateLimiter` port, 사용자 1초와 (사용자, 방송) 슬로우 모드 슬롯을 한 번에 전부 잡거나 하나도 안 잡는다. 방송자는 슬로우 모드 면제. 슬로우 모드는 `broadcasts.slow_mode_seconds`(0~300)이고 방송 `version`을 올리지 않는다 | 제목 수정 폼과 충돌하지 않는 독립 토글이다. 실패한 시도도 슬롯을 쓴다(리뷰 F6 거부): 시도 횟수 기준 제한이고, 종료된 방송은 슬롯을 잡기 전에 걸러진다 |
| 작성은 `INSERT ... SELECT FROM broadcasts WHERE status = ANY(열린 상태)`, 수정·삭제는 `author_id`, `version` 조건부 UPDATE 한 문장. 종료된 방송에서도 작성자는 수정·삭제할 수 있다 (리뷰 F16) | 확인과 변경 사이에 끼어들 틈이 없다. 방송이 끝나면 작성만 닫고, 작성자가 자기 글을 정리할 수는 있게 둔다 |
| 웹은 본문을 React 텍스트 자식으로만 렌더하고 oxlint `react/no-danger`를 켠다. SSE `data`는 형식 검사 후 쓰고, 틀리면 목록을 다시 읽는다 | 완료 조건 "React 기본 이스케이프만". 헤드리스 Chrome에서 `<script>` 본문이 글자로 보이고 실행되지 않음을 확인 |

## Rejected alternatives

- 커밋 뒤 `EventBus::publish`로 따로 발행: 두 쓰기의 알림 순서가 뒤집혀 재연결 시 낮은 revision을 잃는다. 발행 실패도 다른 인스턴스에서 조용히 사라진다.
- outbox 테이블 + 릴레이: 같은 보장을 더 많은 부품으로 얻는다.
- 알림에는 ID만 보내고 인스턴스가 다시 읽기: 1인스턴스 규모에서 이득이 없고 쿼리가 늘어난다. payload 한도는 측정으로 여유가 확인됐다.
- 이벤트 id를 댓글 Snowflake id로: 수정과 삭제를 재생할 수 없다.
- lock을 기다리기 전에 프로세스 안 방송별 mutex로 먼저 줄 세우기 (리뷰 F25 거부): 지금 규모(t3.medium 1대, 풀 10)에서 한 방송에 동시 쓰기 10개가 겹쳐야 풀이 찬다. 처리량 문제가 보이면 묶음 INSERT(comment/1의 확장 경로)와 함께 다시 본다.
- `Missed` 뒤 정책을 방송별로 한 번만 읽기 (리뷰 F26 거부): listener 재연결은 드물고, 팔로워별 따라잡기 쿼리가 이미 있다.
- 웹 jsx-a11y 플러그인 전체 적용 (리뷰 F3 거부): 켜 보면 `prefer-tag-over-role`만 나오고 대부분 기존 화면이다. 이슈 범위 밖.

## Cautions when touching this

- 댓글 쓰기는 반드시 `PgCommentRepository::locked`를 지난다. lock 없이 `comments`에 쓰거나 커밋 뒤에 알리면 복구에서 댓글이 빠진다.
- #13 도네이션처럼 저장하고 순서대로 전달해야 하는 쓰기는 같은 방식으로 트랜잭션 안에서 `realtime::notify`를 부르고, 새 이벤트는 `EventKind`와 `realtime`의 wire 형식에 variant를 더한다. 피드와 SSE 배치(`FeedItem`, `Follower`)도 함께 넓힌다.
- `PgEventBus`는 풀 연결 하나를 계속 쥔다. 풀 크기를 줄이면 이를 계산한다.
- 속도 제한과 스트림 수는 인스턴스 메모리다. 인스턴스가 늘면 근사치다(comment/1).
- 웹 입력 `maxLength`는 UTF-16 단위라 이모지에서는 서버(문자 수)보다 엄격하다.
- (unverified) 한 방송의 쓰기 처리량 상한(lock 직렬화)은 측정하지 않았다.
