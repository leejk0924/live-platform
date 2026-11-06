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
| data | [1-concurrency-snowflake](data/1-concurrency-snowflake.md) | 조건부 UPDATE·version·UNIQUE·advisory lock, Snowflake ID |
| data | [4-snowflake-worker-lease](data/4-snowflake-worker-lease.md) | Snowflake 상태 머신·PG worker lease·에러 응답 봉투 |
| auth | [1-hybrid-session-jwt](auth/1-hybrid-session-jwt.md) | 웹은 DB 세션 쿠키, 송출은 1회용 단기 JWT |
| media | [1-mediamtx-whip-llhls](media/1-mediamtx-whip-llhls.md) | MediaMTX에 미디어 중계, WHIP 송출, LL-HLS·WHEP 시청 |
| comment | [1-realtime-sse-comments](comment/1-realtime-sse-comments.md) | REST 쓰기 + SSE push, 샘플링 없이 쓰기 측 제어 |
| infra | [1-aws-single-ec2-compose](infra/1-aws-single-ec2-compose.md) | EC2 t3.medium 1대 + compose, 컨테이너 스로틀링 |
