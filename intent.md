# intent.md — 무엇을, 왜 만드는가

> 한 장짜리 북극성 문서. 결정의 상세 근거는 [`docs/adr/`](docs/adr/README.md)에 있다.
> 확정일: 2026-11-05 (grilling 세션 합의)

## 1. 목표

치지직(chzzk.naver.com)과 같은 **실시간 방송 플랫폼**을 만든다.

- 필수: PC 화면 공유 방송(탭/시스템 소리 + 마이크), 모바일 카메라 방송, 방송 CRUD·목록·상세·시청, 상태 관리(방송 중/종료), 영구 저장, 회원가입·로그인·로그아웃, 소유자 인가.
- 선택(가산점): 댓글 → 녹화·다시보기 → 도네이션 순으로 구현.
- 마감: 2026-11-19 23:59. **구현 일정은 7일(11-05 ~ 11-11)**, 이후는 버퍼.

## 2. 비목표 (지금 하지 않는 것)

- 네이티브 앱. PC·모바일 모두 반응형 웹앱 하나로 처리한다.
- 트랜스코딩(ABR). 지금은 passthrough + 송출 측 720p30 / 2.5Mbps 제한.
- 다중 노드 운영, CDN 라이브 송출, WAF/Shield. **설계는 열어두되** 구현하지 않는다.
- 댓글 샘플링. 대규모 채팅은 쓰기 측 제어로 다룬다.

## 3. 규모 가정 (설계 방향성)

| 구분 | 방송인 수 | 평균 동시 시청 |
|---|---|---|
| 소형 | ~1,000 | ≤ 1,000 |
| 중형 | ~100 | 3,000 ~ 7,000 |
| 대형 | ~25 | ≥ 10,000 |
| 이벤트 피크 | 드묾 | **500만** |

→ 평상시 동시 시청 ~130만, 피크 500만, 피크 송출 대역 ~15 Tbps.
→ 시청은 **LL-HLS + CDN**으로만 확장 가능하다. 구현은 1대지만 구조는 이 규모로 확장 가능해야 한다.

## 4. 핵심 원칙

1. **Hexagonal (Ports & Adapters)**: `domain` / `application` / `infrastructure` / `api` crate. 의존 방향은 안쪽으로만. 인프라·프레젠테이션은 미정이므로 전부 adapter다.
2. **SOLID**: 특히 ISP(작은 trait)와 DIP(`Arc<dyn Trait>` 주입). `domain`·`application`에는 AWS, MediaMTX, axum 같은 단어가 나오지 않는다.
3. **5개 plane 분리**: Control / Ingest / Delivery / Realtime / Data. 각 plane은 "무엇에 비례해 늘어나는가"로 독립 확장한다.
4. **상태의 진실은 DB**: 메모리 락을 source of truth로 쓰지 않는다. 상태 전이는 조건부 UPDATE(CAS), 중복은 UNIQUE 제약으로 막는다. API 인스턴스는 무상태.
5. **보안은 deny by default**: 허용 경로만 명시. 인가는 유스케이스 계층에서, 소유자 확인과 실행은 한 SQL 문장으로.
6. **학습 가능성**: 작성자는 Rust 학습 중인 Java 개발자다. 모든 핵심 코드에는 Java 관점 설명이 Obsidian에 있다.

## 5. 우선순위

```
필수 기능 + 배포 가능 상태  >  인가·보안 정확성  >  코드 구조  >  선택 기능  >  확장 구현
```

일정이 밀리면 도네이션 → 녹화 → 댓글 순으로 버린다. 필수 기능과 배포는 버리지 않는다.

## 6. 배포

- AWS 서울 리전 EC2 t3.medium 1대 + docker compose(Caddy, Rust API, MediaMTX, Postgres) + duckdns + Let's Encrypt.
- 로컬에서도 `docker compose up` 한 번으로 동일하게 동작해야 한다(README 실행 방법).
- 컨테이너별 CPU·메모리·PID·로그 크기 제한(스로틀링)을 둔다.

## 7. 작업 방식

Claude가 구현하고 Java 관점 설명을 붙인다 → 사용자가 리뷰한다 → Claude가 수정한다.
작업 단위가 끝날 때마다 ADR(`docs/adr/`)과 Obsidian(Rust-Learning / Devlog)을 갱신한다.
