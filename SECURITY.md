# 보안 정책

## 취약점 신고

공개 이슈에 올리지 말고 GitHub의 비공개 취약점 신고(저장소 `Security` 탭 → `Report a vulnerability`)로 알려 주세요.
재현 절차, 영향 범위, 응답의 `traceId`가 있으면 함께 적어 주세요. 지원하는 버전은 `main` 하나입니다.

## 원칙

deny by default. 허용하는 경로만 명시하고, 새 라우트도 기본으로 같은 방어를 받게 만든다.
인가는 유스케이스에서, 소유자 확인과 실행은 SQL 한 문장으로 한다. 설계 근거는 [`docs/adr/`](docs/adr/README.md).

## 위협과 대응 코드·테스트

Threat Model의 영역(S1~S10)마다 막는 코드와 그것을 고정한 테스트를 적는다. 테스트 이름은 파일 안의 함수 이름이다.
DB 테스트는 `#[ignore]`라 `cargo test --workspace -- --include-ignored`로 함께 돈다. 운영 스택은 `deploy/check-hardening.sh`로 확인한다.

### S1. 공격 표면 전반 (횡단)

| 위협 | 대응 | 코드 | 검증 |
|---|---|---|---|
| 내부 정보 노출 | 모든 에러는 `{code, message, traceId}`만, 500은 원인을 로그에만 | `crates/api/src/error.rs` | `api::error` 단위 테스트 `internal_error_hides_the_detail_and_logs_it`, `api::lib` `handler_panic_is_an_internal_envelope_without_the_payload` |
| 부인(Repudiation) | 감사 로그: target `audit`, 이벤트 `login_failed` `forbidden` `publisher_kicked` `kick_failed` `media_access_denied`, 결제 이벤트(S6). 요청 span에 `trace_id` `client` `user` | `crates/api/src/audit.rs`, `crates/api/src/error.rs` | `tests/request_logging.rs` `a_forbidden_response_is_one_audit_line_with_the_request_context`, `tests/audit_log.rs` `refused_logins_are_audited_without_the_password`, `a_forbidden_answer_is_audited_with_the_signed_in_user`, `an_owner_ending_a_broadcast_audits_the_kick`, `a_username_that_cannot_be_one_stays_out_of_the_audit`, `a_stream_of_an_ended_broadcast_audits_the_hook_kick`, `a_refused_publish_token_is_audited_without_the_token`(결제 이벤트는 S6), `api::reaper` `report_pass_audits_each_kicked_broadcast_except_failed_ones` |
| 느린 연결·큰 요청(DoS) | API 요청 10초, 본문 64KB. Caddy가 헤더 10초·본문 30초 안에 오지 않는 연결과 32KB 넘는 헤더를 끊는다 | `crates/api/src/lib.rs`, `deploy/caddy/Caddyfile` | `api::lib` `bounded_request_past_the_time_limit_is_a_timeout_envelope`, `body_over_the_limit_is_a_payload_too_large_envelope`, `deploy/check-hardening.sh` |

### S2. 로그인·세션

| 위협 | 대응 | 코드 | 검증 |
|---|---|---|---|
| 비밀번호 유출 후 크래킹 | argon2id, `spawn_blocking` + 세마포어, 10자 이상 + 유출 목록 거부 | `crates/infrastructure/src/auth.rs`, `crates/domain/src/auth.rs` | `tests/auth.rs` `the_password_is_stored_as_an_argon2id_hash`, `signup_rejects_taken_reserved_and_weak_input` |
| 세션 탈취·고정 | 256bit 토큰의 SHA-256만 저장, `__Host-` HttpOnly Secure SameSite=Lax, 로그인 시 재발급, 유휴 30분·절대 7일 | `crates/infrastructure/src/persistence/session.rs`, `crates/api/src/auth.rs` | `tests/auth.rs` `the_database_holds_only_the_token_sha256`, `signup_signs_in_with_a_hardened_session_cookie`, `login_issues_a_new_token_and_revokes_the_one_sent`, `sessions_expire_after_30_idle_minutes_or_7_days` |
| 계정 열거 | 없는 계정·틀린 비밀번호 모두 해시 검증 한 번, 같은 에러 | `crates/application/src/auth.rs` | `tests/auth.rs` `unknown_user_and_wrong_password_are_indistinguishable` |
| 크리덴셜 스터핑 | IP당 분 10회, 사용자명당 분 5회 (인스턴스 메모리 GCRA) | `crates/api/src/rate_limit.rs` | `tests/auth.rs` `repeated_logins_for_one_username_are_rate_limited`, `api::rate_limit` `login_is_limited_per_username_across_clients` |

미구현: 실패 횟수에 따른 점진 지연. 속도 제한으로 대신한다.

### S3. 브라우저

| 위협 | 대응 | 코드 | 검증 |
|---|---|---|---|
| CSRF | SameSite=Lax에 더해 허용 `Origin` 정확히 일치 + `X-Requested-With: live-platform`. `/api` 전체에 미들웨어 | `crates/api/src/csrf.rs` | `tests/auth.rs` `state_changes_without_origin_and_marker_header_are_rejected` |
| XSS | `dangerouslySetInnerHTML` 미사용, CSP `script-src 'self'`(인라인 스크립트 금지), `object-src 'none'`, `base-uri 'none'` | `deploy/caddy/Caddyfile` | `deploy/check-hardening.sh` |
| 클릭재킹, MIME 스니핑, 정보 누출 | `frame-ancestors 'none'`, `X-Content-Type-Options: nosniff`, `Referrer-Policy: strict-origin-when-cross-origin`, `Server` 헤더 제거 | `deploy/caddy/Caddyfile` | `deploy/check-hardening.sh` |
| HTTPS 다운그레이드 | 운영 HSTS 1년(`compose.prod.yml`). 로컬은 내부 CA를 손으로 허용해야 해서 0 | `deploy/caddy/Caddyfile` | `deploy/check-hardening.sh prod` |
| 기기 권한 남용 | `Permissions-Policy: camera=(self), microphone=(self), display-capture=(self), geolocation=()` | `deploy/caddy/Caddyfile` | `deploy/check-hardening.sh` |

### S4. 인가

| 위협 | 대응 | 코드 | 검증 |
|---|---|---|---|
| 남의 방송 수정·삭제(IDOR) | `WHERE id AND owner_id AND status` 조건부 UPDATE 한 문장, 유스케이스에서 인가 | `crates/infrastructure/src/persistence/broadcast.rs`, `crates/application/src/broadcast.rs` | `tests/broadcast_authz.rs` `authorization_matrix`, `tests/broadcast_live.rs` `authorization_and_status_matrix` |
| 대량 할당 | 요청 DTO가 `owner_id`·`status`·`version`을 받지 않는다 | `crates/api/src/broadcast.rs` | `tests/broadcast_authz.rs` `create_binds_owner_status_and_version_from_the_server_only` |
| SQL 인젝션 | sqlx 바인딩만 | `crates/infrastructure/src/persistence/` | 코드 리뷰 규칙(`CLAUDE.md`) |
| 입력 크기 | 제목 100자, 설명 2000자, 본문 64KB | `crates/domain/src/broadcast.rs` | `infrastructure/tests/broadcast_schema.rs` `the_table_accepts_exactly_what_the_domain_accepts` |

### S5. 미디어

| 위협 | 대응 | 코드 | 검증 |
|---|---|---|---|
| 남의 방송으로 송출(하이재킹) | EdDSA JWT, TTL 60초, 경로 한정, `jti` 1회용, 송출 허가를 MediaMTX가 API에 묻는다 | `crates/infrastructure/src/media/jwt.rs`, `crates/api/src/internal.rs` | `tests/media_admission.rs` `a_token_publishes_only_to_its_own_broadcast`, `fifty_concurrent_uses_of_one_token_admit_one` |
| 가짜 송출 시작·종료 이벤트 | `/internal/*`은 공유 비밀값(상수 시간 비교), 공개 `/api` 밖 | `crates/api/src/internal.rs` | `tests/media_hooks.rs` `hooks_need_the_secret_and_a_known_shape`, `tests/media_admission.rs` `the_internal_routes_are_not_under_the_public_api` |
| MediaMTX 관리 API 노출 | Control API는 내부망 전용, JWT 필요, 외부 포트 없음 | `compose.yml`, `deploy/mediamtx/mediamtx.yml` | `deploy/README.md` 1절 포트 표 |

미구현: 녹화 서명 URL(#12).

### S6. 결제

| 위협 | 대응 | 코드 | 검증 |
|---|---|---|---|
| 금액 위·변조 | 확인 요청의 금액이 저장된 주문 금액과 다르면 Toss를 부르지 않고 거부. Toss 승인 결과의 `totalAmount`, `orderId`, `paymentKey`, `status`도 주문과 대조한 뒤에만 적립 | `crates/application/src/donation.rs` | `application::donation` `an_amount_other_than_the_orders_never_reaches_the_gateway`, `an_approval_that_differs_from_the_order_credits_nothing`, `tests/donations.rs` `a_confirmation_must_match_the_order_its_donor_and_tosss_answer` |
| 중복 적립·이중 결제 | `UPDATE ... WHERE status = 'PENDING'` 한 문장과 합계 upsert, 알림을 한 트랜잭션에서. 같은 주문의 Toss 확인은 늘 같은 `Idempotency-Key` | `crates/infrastructure/src/persistence/donation.rs`, `crates/infrastructure/src/payment/toss.rs` | `infrastructure/tests/donation_store.rs` `ten_concurrent_confirmations_credit_the_order_once`, `payment::toss` `confirm_posts_the_order_with_basic_auth_and_the_idempotency_key` |
| 한 결제로 여러 주문 채우기 | `payment_key` UNIQUE, 위반은 `ALREADY_PAID` | `crates/infrastructure/migrations/0009_donations.sql` | `infrastructure/tests/donation_store.rs` `a_payment_pays_for_one_order_only` |
| 위조 webhook | 본문은 `data.paymentKey`만 읽고 Toss 조회 API(`GET /v1/payments/{paymentKey}`)로 다시 확인한다. 키는 `[A-Za-z0-9_-]` 1~200자만 받아 URL 경로를 벗어날 수 없다 | `crates/api/src/donation.rs`, `crates/domain/src/donation.rs` | `application::donation` `a_recheck_credits_only_what_the_gateway_reports_paid_in_full`, `domain::donation` `payment_key_is_1_to_200_url_safe_ascii_characters`, `tests/donations.rs` `the_webhook_credits_what_toss_reports_paid` |
| 남의 주문 결제·자기 후원 | 주문의 후원자만 확인 가능(`NOT_DONOR`), 방송 소유자는 주문 불가(`SELF_DONATION`, INSERT 조건에도) | `crates/application/src/donation.rs`, `crates/infrastructure/src/persistence/donation.rs` | `tests/donations.rs` `orders_are_refused_to_the_owner_off_air_and_out_of_range`, `infrastructure/tests/donation_store.rs` `an_order_is_stored_only_for_a_viewer_of_a_broadcast_on_air` |
| 키 유출·라이브 결제 | `TOSS_SECRET_KEY`는 `Secret`, Debug와 에러에 없음. `test_` 키만 받고 아니면 시작하지 않는다 | `crates/api/src/config.rs`, `crates/infrastructure/src/payment/toss.rs` | `api::config` `live_toss_keys_are_refused_without_echoing_them`, `payment::toss` `debug_never_shows_the_secret_key`, `an_unreachable_toss_is_unavailable_without_the_key_or_the_secret` |
| 결제 부인 | 감사 이벤트 `donation_ordered` `payment_confirmed` `payment_rejected`(거절된 확인마다 `reason`) `payment_amount_mismatch` `payment_webhook_checked`. 결제 키·카드 정보·webhook 본문은 남기지 않는다 | `crates/api/src/donation.rs`, `crates/api/src/audit.rs` | `tests/audit_log.rs` `payments_are_audited_without_the_payment_key`, `webhook_rechecks_are_audited_without_the_payment_key`, `an_approved_payment_that_is_not_credited_is_audited_without_the_key` |

미구현: 결제 취소·환불, 승인됐지만 주문과 맞지 않아 적립하지 않은 결제의 자동 취소. 운영 키(라이브)는 받지 않는다.

### S7. 남용

| 대상 | 한도 | 코드 | 검증 |
|---|---|---|---|
| 로그인 | IP당 분 10, 사용자명당 분 5 | `crates/api/src/rate_limit.rs` | `tests/auth.rs` `repeated_logins_for_one_username_are_rate_limited` |
| 회원가입 | IP당 10분 5 | `crates/api/src/rate_limit.rs` | `api::rate_limit` `signup_allows_five_per_client_then_says_when_to_retry` |
| 결제 webhook | 클라이언트당 분 60(넘으면 Toss 조회 전에 429, Toss가 재전송) | `crates/api/src/rate_limit.rs` | `api::donation` `the_webhook_is_rate_limited_per_client_before_asking_toss` |
| 후원 주문 | 사용자당 3초에 1건(`DONATION_ORDER_INTERVAL`) | `crates/application/src/donation.rs` | `application::donation` `orders_are_spaced_out_per_donor`, `tests/donations.rs` `a_donor_orders_again_only_after_a_pause` |
| 동시 방송 | 사용자당 활성 방송 1개(부분 UNIQUE 인덱스) | `crates/infrastructure/migrations/` | `infrastructure/tests/broadcast_schema.rs` `an_owner_holds_at_most_one_broadcast_in_the_active_statuses` |
| 컨테이너 자원 | CPU·메모리·PID·로그 크기 제한 | `compose.yml` | `docker compose config` |

미구현: 방송 시작 분당 5회 제한, 댓글·SSE 제한(#11). AWS WAF/Shield와 분산 속도 제한은 ADR에만 둔다.

### S8. 인프라·공급망

| 위협 | 대응 | 코드 | 검증 |
|---|---|---|---|
| 컨테이너 탈출·변조 | 모든 서비스 non-root, read-only rootfs, `cap_drop: ALL`, `no-new-privileges`. api는 distroless, caddy는 셸 없는 distroless, mediamtx는 scratch 기반 | `compose.yml`, `docker/` | `deploy/check-hardening.sh` |
| 이미지 바꿔치기 | 모든 베이스 이미지와 compose 이미지를 digest로 고정 | `docker/*.Dockerfile`, `compose.yml` | `grep -n '^FROM\|image:' docker/*.Dockerfile compose.yml` |
| 내부 서비스 노출 | Postgres, API, MediaMTX Control API는 외부 포트 없음 | `compose.yml` | `deploy/README.md` 1절 |
| 악성·취약 의존성 | cargo-deny(라이선스, RustSec, 출처, 계층 금지 crate), npm audit(high 이상), lockfile 커밋, Dependabot 주간 갱신 | `deny.toml`, `.github/workflows/ci.yml`, `.github/dependabot.yml` | CI `cargo-deny`, `web` job |
| 비밀값 커밋 | `.env` 무시, `.env.example`은 자리표시자만, gitleaks pre-commit hook과 CI | `.gitignore`, `.githooks/pre-commit`, `.github/workflows/ci.yml` | CI `gitleaks` job |

미적용: IMDSv2, IAM Role, SSM(AWS 배포 #2에서), Secrets Manager, cosign.

### S9. 데모 계정

| 위협 | 대응 | 코드 | 검증 |
|---|---|---|---|
| 데모 계정으로 일반 계정 로그인 | 비밀번호 없는 계정(`Credential::DemoOnly`), `demo-` 접두사 가입 예약, `DEMO_LOGIN_ENABLED` 기본 꺼짐 | `crates/domain/src/auth.rs`, `crates/api/src/config.rs` | `tests/auth.rs` `demo_login_is_hidden_while_disabled`, `demo_login_signs_into_seeded_accounts_that_have_no_password` |
| 데모 데이터 오염 누적 | 오늘(KST) 이전에 만든 데모 방송을 reaper가 soft delete | `crates/infrastructure/src/persistence/reaper.rs` | `infrastructure/tests/reaper.rs` `demo_broadcasts_from_before_today_are_reset_once_they_are_not_active` |

### S10. 로그·에러

| 위협 | 대응 | 코드 | 검증 |
|---|---|---|---|
| 로그에 비밀값 | `Secret<T>`의 Debug는 가리고 Display는 없다. 설정 로그는 DB URL을 가린다. 감사 로그에 비밀번호를 남기지 않는다 | `crates/domain/src/secret.rs`, `crates/api/src/config.rs` | `domain::auth` `password_debug_is_redacted`, `api::config` `logging_the_config_redacts_the_database_url`, `tests/audit_log.rs` `refused_logins_are_audited_without_the_password` |
| 추적 불가 | 모든 요청에 `x-trace-id`, 로그 span에 같은 값 | `crates/api/src/error.rs` | `tests/request_logging.rs` `logs_inside_a_request_carry_its_trace_id` |
