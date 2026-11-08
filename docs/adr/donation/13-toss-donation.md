---
issue: 13
area: donation
summary: 후원은 donation id를 Toss orderId로 쓰는 PENDING 주문, 주문 단위 Idempotency-Key 승인, 조건부 UPDATE 한 트랜잭션의 적립·합계·pg_notify로 한 번만 적립하고, webhook은 paymentKey 조회로만 재확인하며, Toss CSP는 /pay 문서에만 연다
status: confirmed
date: 2026-11-09
paths: [crates/domain/src/donation.rs, crates/application/src/donation.rs, crates/application/src/ports/donation.rs, crates/application/src/ports/payment.rs, crates/infrastructure/src/payment, crates/infrastructure/src/persistence/donation.rs, crates/infrastructure/migrations/0009_donations.sql, crates/infrastructure/src/realtime.rs, crates/api/src/donation.rs, crates/api/src/comment_stream.rs, deploy/caddy/Caddyfile, web/src/donations]
related: [1, 11, 14]
---

# 도네이션: Toss 테스트 모드 결제와 한 번만 적립

## Background

시청자가 방송에 Toss Payments 결제위젯(테스트 모드)으로 후원하고, 서버가 금액과 결과를 검증한 뒤 한 번만 적립해야 한다. 같은 승인이 여러 번(더블 클릭, 재시도, webhook) 와도 적립은 1회여야 하고, 후원은 #11의 실시간 경로로 바로 보여야 한다. 결제 감사 로그와 Toss 위젯용 CSP는 #14에서 넘어온 일이다.

## Decision and reasons

| 결정 | 이유 |
|---|---|
| 주문은 `donations` 행(PENDING)이고 Toss `orderId`는 donation id(Snowflake 10진 문자열)다. 생성은 `INSERT ... SELECT FROM broadcasts WHERE status = ANY(LIVE, INTERRUPTED) AND owner_id <> 후원자` 한 문장 | 별도 주문 번호 컬럼이 필요 없다. 셀프 후원과 종료된 방송을 유스케이스에서 먼저 막고, 확인과 쓰기 사이의 틈은 SQL 조건이 막는다(data/1) |
| confirm 순서: 후원자 본인 확인 → 요청 금액 = 저장 금액(다르면 Toss를 부르지 않음) → 이미 DONE이면 같은 paymentKey일 때 같은 결과 → Toss 승인(`Idempotency-Key = donation-confirm-<orderId>`) → 응답의 orderId·paymentKey·금액·DONE 대조 → `settle` | 금액은 클라이언트 값이 아니라 서버 주문이 진실이다. 키를 주문에서 결정적으로 만들어, 10초 요청 상한에 잘린 confirm을 다시 보내도 Toss가 같은 승인 결과를 돌려준다(실측: 같은 키 재요청 200, 같은 결과) |
| `settle`은 트랜잭션 하나: `UPDATE ... WHERE id=$1 AND status='PENDING'`(0행이면 NotPending, `payment_key` UNIQUE 위반이면 PaymentKeyInUse) → `donation_totals` UPSERT → `realtime::notify`(priority) → commit | 동시 confirm 10개 중 하나만 행을 바꾼다(실제 DB 테스트 `ten_concurrent_confirmations_credit_the_order_once`, 조건을 깨면 실패함을 확인). 합계 행 잠금이 한 방송의 적립을 줄 세워 알림 속 합계가 커지기만 한다 |
| 같은 Idempotency-Key의 동시 confirm에 Toss는 첫 요청 외에 409 `IDEMPOTENT_REQUEST_PROCESSING`을 준다(테스트 환경에서 5건 동시 전송으로 실측: 200 1건, 409 4건). 이 코드는 재시도 가능한 503으로 답하고, 다른 4xx 거절 뒤에도 주문을 다시 읽어 같은 키로 DONE이면 `credited:false`로 성공 응답 (리뷰 F3, F24) | 다른 요청이 이미 적립했거나 적립 중인데 사용자에게 결제 실패를 보이지 않는다. 다시 보내면 Toss가 첫 요청의 결과를 돌려준다 |
| webhook(`POST /api/payments/toss/webhook`)은 본문에서 `paymentKey`만 꺼내 Toss 조회 API(`GET /v1/payments/{paymentKey}`)로 상태·금액을 다시 확인하고 같은 `settle`로 적립한다. CSRF guard 밖, 10초·64KB 상한 안, 클라이언트당 분당 60회 (리뷰 F5) | Toss 결제 상태 webhook은 서명이 없어 본문을 믿을 수 없다. 조회는 우리 비밀 키로 나가므로 남용을 제한한다. 저장소·Toss 실패는 503으로 답해 Toss가 다시 보내게 한다 |
| orderId 조회(`GET /v1/payments/orders/{orderId}`)는 쓰지 않는다 | 사용자가 준 키(Toss 문서 공용 테스트 상점 `tgen_docs`의 위젯 키)로는 실제 DONE 결제의 orderId도 404 `NOT_FOUND_MERCHANT`였다. 같은 결제의 paymentKey 조회는 200 DONE. 원인은 이 상점·키 조합으로 추정(미확인) |
| `PaymentGateway` port(`confirm`, `lookup`)에 Toss 이름을 넣지 않는다. Toss 어댑터는 8초 타임아웃, 4xx `{code}`(카드 거절 403 포함)는 Rejected, 401·408·429·409 처리 중·5xx·네트워크는 Unavailable(503) (리뷰 F17, F23) | 401(잘못된 키, 실측 `UNAUTHORIZED_KEY`)은 사용자 카드 문제가 아니라 설정 문제다. Toss는 카드 거절을 403으로 주므로 403은 거절로 둔다. 테스트는 가짜 TCP 서버로 요청 줄·Basic 인증·Idempotency-Key·본문을 확인한다 |
| reqwest에 `native-tls`를 켠다 | 기본 기능 없이 써서 TLS가 없었고 컨테이너에서 Toss 호출이 실패했다(가짜 게이트웨이 테스트로는 안 보임). rustls는 aws-lc와 ISC·CDLA 라이선스를 deny 허용 목록에 더해야 했다. distroless/cc 이미지에서 실제 승인이 성공함을 e2e로 확인 |
| Toss 키는 `TOSS_CLIENT_KEY`, `TOSS_SECRET_KEY`(둘 다 `Secret`). 둘 다 비면 후원 비활성(503 `PAYMENTS_DISABLED`), 하나만 있거나 `test_`로 시작하지 않으면 시작 거부 | 테스트 모드만 허용한다는 약속을 코드로 고정한다. 클라이언트 키는 공개 값이라 주문 생성 응답(`clientKey`)으로 내려준다. 빌드 시 주입은 키를 바꿀 때마다 웹을 다시 빌드해야 한다 |
| 실시간은 #11 경로를 재사용: `EventKind::Donation`, `FeedItem::Donation`(커서와 무관, 즉시 flush), SSE `donation`·`donations` 이벤트. 합계 요약은 피드를 열 때와 따라잡은 뒤 policy 다음에 보낸다. 클라이언트는 합계를 최대값으로 유지하고, 요약의 count가 목록보다 크면 목록을 다시 읽는다 | 새 SSE 엔드포인트 없이 우선 레인을 쓴다. 후원은 댓글 revision 커서에 없으므로 놓친 개별 알림은 합계와 목록 재조회로 메운다 |
| Toss 출처는 `/pay`, `/pay/*` 문서의 CSP에만 더한다(`header @pay >Content-Security-Policy`). 결제는 SPA 라우터가 아니라 전체 페이지 이동으로 `/pay/checkout`에 들어간다 | CSP는 문서 단위라 다른 화면은 엄격한 정책 그대로다. 출처 목록은 헤드리스 Chrome으로 위젯 렌더, requestPayment, 퀵계좌이체 전체 결제, 카드·토스페이·카카오페이·네이버페이 창 열기까지 위반 0을 확인해 정했다. 서버가 라이브 키를 거부하므로 sandbox 호스트만 연다 |
| 감사 로그(`target: audit`): `donation_ordered`, `payment_confirmed`, `payment_rejected`(거절된 모든 confirm, `reason`. 403은 미들웨어의 `forbidden`만, 리뷰 F26), `payment_amount_mismatch`(`at=request|gateway`), `payment_webhook_checked`. paymentKey, 시크릿, 카드 정보, webhook 본문은 남기지 않는다 (리뷰 F4) | common/14 형식. 승인됐지만 적립되지 않은 경우도 추적해야 한다 |
| 주문 생성은 사용자당 3초 간격(`RateKey::DonationOrder`) (리뷰 F7) | 로그인 사용자가 PENDING 행을 무한히 쌓지 못하게 한다 |

## Rejected alternatives

- 후원을 댓글 revision 시퀀스에 넣어 `changes_since`로 재생: 놓친 후원을 정확히 재생하지만 댓글 lock·쿼리 계약을 넓힌다. 합계 요약 + 목록 재조회로 충분하다.
- 합계를 매번 `SUM`: 동시 적립에서 알림 속 합계 순서가 보장되지 않는다.
- Toss 승인 실패 시 주문을 FAILED로: 동시 요청 중 실패 응답이 먼저 오면 성공한 결제를 막을 수 있다. 실패는 주문을 그대로 두고 감사만 남긴다.
- 전역 CSP에 Toss 출처 추가: 결제와 무관한 화면까지 외부 스크립트·iframe을 허용한다.
- 가상계좌 대기(`WAITING_FOR_DEPOSIT`) 처리 (리뷰 F6 거부): 위젯 DEFAULT variant에 가상계좌가 없다(퀵계좌이체·카드·토스페이·페이코·카카오페이·네이버페이).
- 목록을 승인 시각 순으로 (리뷰 F15 거부): id 순서가 커서 페이지네이션 키와 같고 인덱스 하나로 끝난다.

## Cautions when touching this

- 적립은 반드시 `PgDonationRepository::settle`을 지난다. 합계 행과 알림이 같은 트랜잭션에 있어야 한다.
- 새 결제 수단이나 라이브 키를 쓰면 `/pay` CSP의 호스트(sandbox → 운영)와 위반 0을 다시 확인한다.
- 환불·취소(CANCELED)는 처리하지 않는다. Toss가 승인했지만 우리가 적립을 거부한 결제(금액·주문·키 불일치)는 과금된 채 남는다.
- webhook 제한은 클라이언트 IP 기준이라 Toss의 모든 전달이 한 키를 공유한다. 실 트래픽에서는 Toss 발신 IP 허용 목록과 함께 다시 본다.
- (unverified) 카드 거절이 403이라는 것은 Toss 문서 기억에 근거하고 실제 거절 결제로 확인하지 않았다.
- (unverified) orderId 조회의 `NOT_FOUND_MERCHANT` 원인(공용 문서 상점 키 제한)은 추정이다.
