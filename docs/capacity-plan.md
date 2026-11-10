# 확장 설계 (Capacity Plan)

구현은 서버 한 대의 docker compose다. 구조는 아래 규모까지 늘릴 수 있게 나눴다. 이 문서의 숫자는 설계 방향을 정하려고 세운 추정이고 측정값이 아니다. 목표와 원칙은 [`intent.md`](../intent.md) 3~4절, 구조 결정은 [ADR common/1](adr/common/1-hexagonal-five-planes.md)에 있다.

## 1. 규모 가정

| 구분 | 방송인 | 평균 동시 시청 | 동시 시청 합 |
|---|---|---|---|
| 소형 | 약 1,000 | 약 300 (1,000 이하) | 약 30만 |
| 중형 | 약 100 | 약 5,000 | 약 50만 |
| 대형 | 약 25 | 약 20,000 | 약 50만 |
| 평상시 합 | 약 1,125 | | 약 130만 |
| 이벤트 피크 | | | 500만 |

## 2. 가정에서 나온 수치

| 항목 | 평상시 | 피크 | 결론 |
|---|---|---|---|
| 시청 전송 대역(1인 평균 3Mbps) | 약 3.9Tbps | 약 15Tbps | 서버 한 대는 수십 Gbps라 CDN이 필요하다 |
| 시청자 1명의 1시간 전송량(2.5Mbps) | 약 1.1GB | | 100만 명이 1시간 보면 약 1.1PB |
| WebRTC SFU로 모두 시청 | 노드 수백 대 | 노드 수천 대와 cascading | 대규모 시청은 LL-HLS로 한다 |
| 모든 방송 트랜스코딩 | 약 4,500 vCPU | | 선택적 트랜스코딩만 현실적이다 |
| 댓글 쓰기(시청자 1% 참여, 슬로우 모드 5초) | | 약 1만 건/초 | Postgres와 묶음 쓰기로 시작할 수 있다 |
| 목록 조회(5초 캐시) | 오리진 수십 rps | | CDN 캐시로 흡수한다 |

## 3. plane별 확장 경로

서비스를 다섯 plane으로 나누고, plane마다 무엇에 비례해 늘어나는지로 따로 확장한다. 지금은 한 대에 모두 올렸지만 compose 서비스와 코드 경계는 plane 단위다.

| plane | 비례 대상 | 지금 구현 | 확장할 때 |
|---|---|---|---|
| Control (api) | API 요청 수 | api 1개. 세션과 상태가 Postgres에 있어 인스턴스를 늘릴 수 있다. 속도 제한, SSE 연결 수, 로컬 녹화 서명 키만 인스턴스 메모리에 있다 | 로드 밸런서 뒤 N개, 속도 제한은 Redis |
| Ingest | 송출자 수 | MediaMTX 1개. 경로 이름과 송출 주소는 `IngestRouter`가 정한다 | 미디어 노드 풀, `IngestRouter`가 노드를 배정 |
| Delivery | 시청자 수 | MediaMTX가 LL-HLS와 WHEP를 직접 낸다. `PlaybackResolver`가 소스 목록을 순서대로 내리고 playback·목록 응답은 `Cache-Control: public, max-age=5`. 트랜스코딩 없이 송출 측에서 720p30, 2.5Mbps로 제한 | LL-HLS 오리진 + CDN, 선택적 트랜스코딩 |
| Realtime | 댓글을 보는 시청자 수 | SSE, Postgres `LISTEN/NOTIFY`(`EventBus`), 200ms 묶음 전송, 클라이언트당 SSE 20개 | SSE 게이트웨이 N개, Redis나 NATS로 `EventBus` 교체 |
| Data | 읽기가 쓰기보다 훨씬 많다 | Postgres 1개. 쓰기 Repository와 읽기 Query port를 나눴다(`BroadcastQuery` 등). 녹화 파일은 로컬 볼륨 또는 S3 + CloudFront | primary + replica, 읽기 캐시 |

## 4. 인스턴스 밖으로 뺀 상태

api를 여러 개 띄우려면 상태가 인스턴스 메모리에 있으면 안 된다. 상태마다 port를 두고 adapter만 바꾸면 되게 했다.

| 상태 | port | 지금 adapter | 확장할 때 |
|---|---|---|---|
| 로그인 세션 | `SessionStore` | `PgSessionStore` (Postgres) | Redis |
| 송출 권한 | `PublishAuthorizer`, `PublishTicketLedger` | EdDSA JWT(`JwtPublishAuthorizer`), 1회용 `jti` 기록은 `PgPublishTicketLedger` | 그대로 |
| 실시간 이벤트 | `EventBus` | `PgEventBus` (`LISTEN/NOTIFY`) | Redis, NATS |
| 미디어 노드 위치 | `IngestRouter`, `PlaybackResolver` | MediaMTX 1개 고정 | 노드 레지스트리, CDN 주소 |
| Snowflake worker id | `WorkerIdAllocator` | `PgWorkerIdAllocator` (Postgres lease) | Redis, StatefulSet 순번 |
| 댓글·후원 속도 제한 | `RateLimiter` | `InMemoryRateLimiter` (인스턴스별 근사) | Redis |
| 주기 작업(reaper) 단일 실행 | `ReaperStore` | `PgReaperStore` (`pg_try_advisory_xact_lock`) | 그대로 |
| 녹화 파일 | `RecordingStorage` | `FsRecordingStorage`, `S3RecordingStorage` | S3 + CloudFront |

가입·로그인 속도 제한은 port 없이 api 안의 `governor`가 인스턴스마다 센다. 여러 인스턴스에서는 한도가 인스턴스 수만큼 늘어난다.

## 5. 댓글 저장 확장 단계

1. 한 건씩 INSERT한다. 지금 단계이고, 방송별 advisory lock 아래에서 쓴다.
2. 50ms 단위로 묶어 INSERT하거나 COPY한다. 초당 수만 건.
3. 로그(Kafka, NATS)에 먼저 쓰고 비동기로 저장한다. 초당 수십만 건.
4. 일 단위 시간 파티셔닝을 하고 오래된 파티션은 S3에 보관한다.
5. 쓰기에 특화된 DB(Cassandra, ScyllaDB)로 옮긴다.

Snowflake ID는 DB 없이 만들고 저장은 `CommentRepository` port 뒤에 있다. 단계를 바꿔도 유스케이스 코드는 그대로다.

## 6. 구현하지 않은 것

다중 노드 운영, CDN 라이브 전달, 트랜스코딩(ABR), WAF는 설계만 열어 두고 구현하지 않았다([`intent.md`](../intent.md) 2절). S3 + CloudFront 녹화 adapter는 가짜 서버 테스트로만 확인했고 실제 AWS에서 돌려 보지 않았다. 댓글 쓰기 처리량의 상한도 측정하지 않았다.
