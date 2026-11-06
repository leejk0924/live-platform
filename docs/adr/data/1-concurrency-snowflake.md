---
issue: 1
area: data
summary: 상태의 진실은 DB에 두고 조건부 UPDATE·version·UNIQUE·advisory lock으로 동시성을 처리, ID는 Snowflake
status: confirmed
date: 2026-11-05
paths: [crates/domain/src/broadcast, crates/infrastructure/src/persistence, crates/infrastructure/src/id]
related: []
---

# 동시성 전략과 ID

## Background

API 인스턴스가 여러 대로 늘어나도 안전해야 한다. 경쟁 지점: 방송 시작/종료/삭제 동시 요청, 미디어 hook과 사용자 요청, 결제 승인 중복 통지, reaper 중복 실행. Rust 컴파일러는 데이터 레이스는 막지만 요청 간 논리적 경쟁은 막지 않는다.

## Decision and reasons

| 대상 | 방식 | Java 대응 |
|---|---|---|
| 방송 상태 전이 | 조건부 UPDATE(CAS). 도메인 상태 머신이 판단, DB가 확정 | `@Modifying` + `WHERE status=?` |
| 방송 정보 수정 | `version` 낙관적 락 | `@Version` |
| 결제 중복 통지 | 멱등키 + UNIQUE + `ON CONFLICT DO NOTHING` | `@Column(unique)` |
| reaper 등 주기 작업 | PG advisory lock | ShedLock |

- 소유자 검증도 같은 문장에 넣는다: `WHERE id=$1 AND owner_id=$2 AND status IN (...)`. 확인과 실행 사이 틈(TOCTOU)이 없다.
- **ID는 Snowflake**(timestamp 41 / worker 10 / sequence 12, epoch 2026-01-01, `BIGINT`).
  - worker ID: `WorkerIdAllocator` port, PG lease 테이블(TTL + heartbeat).
  - 시계 역행: 시작 시 벽시계 1회 + 이후 단조 시계(`Instant`)로 진행, lease에 `last_ms` 저장해 재시작 시 `now > last_ms`까지 대기. 큰 역행은 거부(안전장치).
  - JSON에서는 **문자열**. JS number는 2^53까지만 정확하다.

## Rejected alternatives

- 프로세스 내 `Mutex`/`DashMap`: 서버 1대에서만 유효.
- 방송별 Actor(tokio task): 다중 인스턴스에 샤딩·라우팅 필요.
- 비관적 락(`FOR UPDATE`): 대기·데드락 관리 비용, CAS로 충분.
- Redis 분산 락: 인프라 추가, 지금은 PG로 충분.
- UUIDv7: 사용자가 Java 경험상 Snowflake(64bit, 정렬, 작은 인덱스)를 선호.
- BIGSERIAL: 생성이 DB에 묶이고 ID 추측이 쉽다.

## Cautions when touching this

- 메모리 상태를 source of truth로 쓰지 않는다. 캐시로만.
- 영향 행 0인 UPDATE는 "권한 없음 / 상태 불일치 / 없음"을 구분해 응답해야 한다.
- Snowflake ID를 숫자로 프론트에 보내면 조용히 깨진다.

## Follow-up (2026-11-06)

"재시작 시 `now > last_ms`까지 대기"는 크래시 경로에서 그대로는 성립하지 않았다. 이슈 #4에서 heartbeat가 실제 `last_ms` 대신 발급 상한을 기록하고, 정상 반납만 정확한 값을 쓰도록 바꿨다. 상세는 [`data/4-snowflake-worker-lease`](4-snowflake-worker-lease.md).
