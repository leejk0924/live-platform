---
issue: 1
area: auth
summary: 웹은 DB 세션 쿠키(SessionStore), 송출은 EdDSA 1회용 단기 JWT를 쓰는 하이브리드 인증
status: confirmed
date: 2026-11-05
paths: [crates/application/src/ports/auth.rs, crates/infrastructure/src/auth.rs, crates/infrastructure/src/persistence/session.rs, crates/infrastructure/src/media/jwt.rs]
related: []
---

# 인증과 송출 권한

## Background

로그인 사용자만 방송 등록·송출, 비로그인도 목록·시청 가능. 수정·삭제·송출 제어는 서버에서 소유자 검증. 인스턴스가 늘어나도 동작해야 하고 로그아웃은 즉시 효력이 있어야 한다.

## Decision and reasons

- **웹 로그인: opaque 세션 토큰 + 쿠키**
  - 256bit 난수, DB에는 SHA-256 해시만 저장(DB 유출 ≠ 세션 탈취).
  - `__Host-` 접두사, `HttpOnly; Secure; SameSite=Lax`, 유휴 30분 / 절대 7일.
  - 로그인 시 세션 재발급(세션 고정 방지).
  - `SessionStore` port: 지금 Postgres, 확장 시 Redis. 공유 저장소라 sticky session 불필요.
- **송출 권한: 단기 JWT capability**
  - API가 소유자·상태 검증 후 발급. EdDSA 비대칭 서명, `kid`로 키 교체, TTL 60초, 특정 경로 publish만, `jti`로 1회용.
  - MediaMTX는 JWKS 공개키로 직접 검증 → 미디어 노드가 늘어도 API 호출 불필요, 노드가 털려도 위조 불가.
- **비밀번호**: argon2id(OWASP 권장 파라미터), 최소 10자, 유출 비밀번호 상위 1만 개 거부. 해시는 `spawn_blocking` + 세마포어.
- **계정 열거 방지**: 동일 에러 메시지, 없는 계정도 가짜 해시 계산. 토큰·HMAC 비교는 상수 시간(`subtle`).
- **데모 계정 2개**(`demo-streamer`, `demo-viewer`): 비밀번호 변경·삭제 불가, 강한 속도 제한, 매일 초기화, `DEMO_LOGIN_ENABLED` 플래그(기본 꺼짐).

## Rejected alternatives

- 순수 JWT(access + refresh): 로그아웃 즉시 무효화 불가, 결국 refresh 저장소 필요.
- 송출 인증을 MediaMTX → API HTTP 콜백만으로: 노드가 늘면 API 부하·가용성 결합.

## Cautions when touching this

- CSRF는 SameSite만 믿지 않는다. 상태 변경 요청은 `Origin` 검증 + 커스텀 헤더 필수.
- 인가는 유스케이스 계층에서. handler 우회 경로가 생기면 안 된다.
- JWT 서명 개인키는 API만 가진다.

## Follow-up (2026-11-08, #7)

송출 토큰 TTL은 60초가 아니라 12시간이다. MediaMTX 1.21.3이 JWT `exp`에 송출 세션을 끊기 때문이다. `jti` 1회용은 강제하지 못한다. 그래서 종료한 방송에 같은 토큰으로 다시 송출할 수 있다. 막는 방법은 #10의 publish hook이다. 자세한 내용은 [media/7-whip-publish-jwt](../media/7-whip-publish-jwt.md).

## Follow-up (2026-11-08, #10)

"송출 인증을 MediaMTX → API HTTP 콜백으로" 하는 방식은 처음에 기각했다. 이 결정을 송출에 한해 뒤집었다. 콜백은 송출 시도에서만 생기고 시청에서는 생기지 않는다. 이렇게 해서 TTL 60초와 `jti` 1회용을 실제로 강제한다. 자세한 내용은 [media/10-publish-admission-reaper](../media/10-publish-admission-reaper.md).

## Follow-up (2026-11-08, 코드 대조)

- 데모 계정 매일 초기화는 아직 없다. auth/5에서 초기화할 데이터가 없다며 미뤘고, 방송이 생긴 지금 #27에서 정한다.
- 데모 로그인 제한은 "강한 제한"이 아니라 일반 로그인과 같은 클라이언트당 분당 10회다(`crates/api/src/rate_limit.rs`).
- frontmatter `paths`를 실제 위치로 고쳤다. 처음 적은 `ports/session.rs`, `infrastructure/src/auth/`는 만들어지지 않았다.

## Follow-up (2026-11-08, #27)

데모 계정 매일 초기화는 "데모 계정의 READY/ENDED 방송 중 KST 오늘 0시 이전에 만든 것을 reaper 패스마다 soft delete"로 정했다. 세션은 폐기하지 않는다. 자세한 내용은 [common/27-request-limits-housekeeping](../common/27-request-limits-housekeeping.md).
