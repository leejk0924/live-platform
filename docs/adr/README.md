# ADR (Architecture Decision Records)

대안을 비교해 내린 설계 결정과 그 이유를 남긴다. 코드가 *무엇*을 하는지는 코드가 말하고, 여기는 *왜 그렇게 했는지*를 말한다.

## 경로

```text
docs/adr/<area>/<issue>-<slug>.md
```

- `area`: `common` `auth` `broadcast` `media` `comment` `recording` `donation` `data` `infra`
- `issue`: 결정이 나온 GitHub 이슈 번호
- `slug`: 영어 소문자, `-` 연결

## 형식

```markdown
---
issue: 6
area: broadcast
summary: 한 줄 요약
status: confirmed          # proposed | confirmed | superseded
date: 2026-11-06
paths: [crates/domain/src/broadcast]   # 이 결정이 걸린 코드
related: [10]                          # 관련 이슈 번호
---

# 제목

## Background
## Decision and reasons
## Rejected alternatives
## Cautions when touching this
```

결정이 바뀌면 파일 끝에 `## Follow-up (YYYY-MM-DD)`를 붙이거나, 새 ADR을 쓰고 기존 것의 `status`를 `superseded`로 바꾼다.

## 목록

| area | ADR | 요약 |
|---|---|---|
| common | [1-hexagonal-five-planes](common/1-hexagonal-five-planes.md) | Hexagonal 4-crate workspace와 5개 plane 분리 |
| common | [25-doc-two-tier-timeline](common/25-doc-two-tier-timeline.md) | ADR은 저장소 요약판과 Obsidian 상세판 두 벌, 읽는 순서는 Obsidian Timeline, 머지 후 Devlog 갱신. Obsidian은 PR 뒤에, 머지 전 확인 필요 재확인(#29) |
| common | [27-request-limits-housekeeping](common/27-request-limits-housekeeping.md) | 요청 10초·본문 64KB 상한, reaper에 만료 세션 정리와 데모 방송 초기화, 정리 실패는 SAVEPOINT로 격리 |
| common | [14-security-hardening](common/14-security-hardening.md) | 보안 헤더·slowloris는 Caddy 한 곳, 감사 로그는 target audit 이벤트(403은 미들웨어), gitleaks pre-commit은 stdin 모드 hook, Caddy distroless |
| data | [1-concurrency-snowflake](data/1-concurrency-snowflake.md) | 조건부 UPDATE·version·UNIQUE·advisory lock, Snowflake ID |
| data | [4-snowflake-worker-lease](data/4-snowflake-worker-lease.md) | Snowflake 상태 머신·PG worker lease·에러 응답 봉투 |
| auth | [1-hybrid-session-jwt](auth/1-hybrid-session-jwt.md) | 웹은 DB 세션 쿠키, 송출은 1회용 단기 JWT |
| auth | [5-session-auth-structure](auth/5-session-auth-structure.md) | Credential enum, adapter 안 토큰 해시, CSRF 미들웨어, 인스턴스별 속도 제한 |
| broadcast | [6-broadcast-crud-authz](broadcast/6-broadcast-crud-authz.md) | 쓰기·읽기 port 분리, 조건부 UPDATE 한 문장 인가, If-Match 낙관적 락, OpenAPI 스냅샷 |
| media | [1-mediamtx-whip-llhls](media/1-mediamtx-whip-llhls.md) | MediaMTX에 미디어 중계, WHIP 송출, LL-HLS·WHEP 시청 |
| media | [7-whip-publish-jwt](media/7-whip-publish-jwt.md) | 시작·종료 조건부 전이, MediaMTX JWT 송출 인증, 12시간 토큰과 재송출 공백 |
| media | [8-playback-player](media/8-playback-player.md) | playback API는 방송 중일 때만 순서 있는 소스 목록, 내장 HLS 없이 hls.js LL-HLS 기본, iOS·저지연은 WHEP, 실패 시 다음 소스 |
| media | [10-publish-admission-reaper](media/10-publish-admission-reaper.md) | 송출 허가를 API(http 인증)로, 1회용 jti, hook과 advisory lock reaper로 INTERRUPTED 30초 유예 |
| recording | [12-recording-replay](recording/12-recording-replay.md) | MediaMTX fMP4 세그먼트를 hook으로 등록하고 lease claim으로 로컬·S3에 저장, 5분 서명 URL(로컬 HMAC 매 요청 확인, CloudFront ECDSA P-256)로 일반 video 재생 |
| comment | [1-realtime-sse-comments](comment/1-realtime-sse-comments.md) | REST 쓰기 + SSE push, 샘플링 없이 쓰기 측 제어 |
| comment | [11-comment-feed-sse](comment/11-comment-feed-sse.md) | 방송별 advisory lock 아래 revision, 같은 트랜잭션의 pg_notify, revision 커서로 Last-Event-ID 복구와 200ms 배치, 클라이언트당 SSE 20개 |
| infra | [1-aws-single-ec2-compose](infra/1-aws-single-ec2-compose.md) | EC2 t3.medium 1대 + compose, 컨테이너 스로틀링 |
