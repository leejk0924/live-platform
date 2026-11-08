---
issue: 1
area: infra
summary: AWS 서울 EC2 t3.medium 1대에 docker compose로 배포하고 컨테이너별 리소스 스로틀링을 적용
status: confirmed
date: 2026-11-05
paths: [compose.yml, compose.prod.yml, deploy/]
related: []
---

# 배포와 컨테이너 스로틀링

## Background

README에 실제 접속 가능한 URL이 필요하고, 화면 공유·카메라는 HTTPS에서만 동작한다. 실사용자는 없고 심사용 데모가 목적이다. 로컬에서도 `docker compose up`으로 동일하게 돌아가야 한다.

## Decision and reasons

- **AWS 서울(ap-northeast-2) EC2 t3.medium 1대**, Elastic IP, duckdns, Caddy(Let's Encrypt 자동 TLS). 월 ~$45. 1일차부터 배포해 실기기로 검증한다.
- **compose 서비스**: caddy, api, mediamtx, postgres. 로컬과 운영은 `compose.prod.yml` 오버레이와 `.env`만 다르다.
- **포트**: 80/443 TCP, 8189 UDP+TCP(WebRTC, TCP는 UDP 차단망 fallback). SSH는 열지 않고 SSM Session Manager 사용. TURN 없음.
- **AWS 보안**: IMDSv2 강제(SSRF로 자격증명 탈취 방지), S3는 IAM Role로만(Access Key 없음).
- **컨테이너 스로틀링**(2 vCPU / 4GB 기준, 1일차에 실측 후 조정)

| 서비스 | cpus | memory | 기타 |
|---|---|---|---|
| caddy | 0.25 | 128M | |
| api | 0.75 | 512M | `pids_limit`, `nofile` 65536(SSE 연결) |
| mediamtx | 1.0 | 768M | `nofile` 65536 |
| postgres | 0.5 | 1G | `shm_size` 256m, `max_connections` 50 |

  - 모든 서비스: `pids_limit`, 로그 로테이션(json-file 10MB × 3), healthcheck, `restart: unless-stopped`, non-root, read-only rootfs, 이미지 digest 고정.
  - cpus 합은 2를 넘지만 상한(ceiling)이다. 메모리 합 ~2.4GB로 OS 여유를 남긴다.
  - 앱 내부 스로틀링: sqlx 풀 최대 10, argon2 동시 실행 세마포어 2, 요청 타임아웃, 본문 64KB, 기능별 `RateLimiter`.
- **t3 CPU 크레딧은 `standard` 모드**: 기준 성능(vCPU당 20%)을 넘는 사용을 과금하지 않고 스로틀링한다. 데모 트래픽에서는 크레딧이 충분히 쌓여 있어 문제가 되지 않고, 예상치 못한 요금을 막는다.

## Rejected alternatives

- 로컬 PC + Cloudflare Tunnel: HTTP만 통과해 외부 WebRTC 송출 불가, PC 상시 가동 필요.
- t3.large: 데모 규모엔 과잉(월 ~$83).
- t4g(ARM): 20% 저렴하지만 arm64 이미지 빌드 리스크.
- RDS: 셋업·비용 증가. `DATABASE_URL` 교체로 나중에 이전 가능.

## Cautions when touching this

- AWS 전용 개념은 `infrastructure` adapter와 `deploy/`에만 둔다.
- MediaMTX는 공인 IP를 ICE 후보로 알려야 한다(Elastic IP 고정 필요).
- Postgres와 MediaMTX API 포트는 외부에 열지 않는다.
- (unverified) 요금과 크레딧 동작은 AWS 문서로 배포 전 확인.

## Follow-up (2026-11-05)

- 과제 문서에 AWS·컨테이너 요구가 없다는 지적으로, "로컬 compose 우선 + AWS는 adapter로만" 구성을 한 번 검토했다(로컬 HTTPS는 Caddy `tls internal`, 녹화는 로컬 파일시스템 + API 서명 URL).
- 비용 추정(t3.medium 월 ~$45, 실사용자 없음)을 보고 사용자가 AWS 기준으로 되돌렸다. 공개 URL과 실기기 HTTPS 검증을 1일차부터 확보하는 쪽이 리스크가 낮다.
- 유지되는 원칙: AWS 전용 개념은 adapter와 `deploy/`에만 두고, 로컬 `docker compose up`도 README 실행 방법으로 계속 지원한다.

## Follow-up (2026-11-06): 검증은 로컬에서만

- 사용자 결정: 테스트는 로컬 `docker compose`에서만 하고, AWS에서는 배포·검증하지 않는다.
- #2는 EC2 없이 할 수 있는 것만 한다: 배포 절차 문서(`deploy/README.md`), `deploy/deploy.sh local|prod`, `compose.prod.yml` 오버레이 점검.
- `deploy.sh local`은 로컬에서 실행해 확인했다. `deploy.sh prod`와 운영 절차는 (unverified).
- 영향: 실기기 화면 공유·카메라는 HTTPS가 필요하다. 공인 HTTPS 주소 없이 #3(실기기 PoC)을 하려면 LAN에서 Caddy 내부 CA를 기기에 신뢰시키거나 다른 터널 수단이 필요하다. #3에서 정한다.
- 위 결정(EC2, IMDSv2, SSM 등)은 지우지 않는다. 운영 서버를 쓰게 되면 그대로 따른다.

## Follow-up (2026-11-08, 코드 대조)

ADR 상세판을 쓰며 main과 대조했더니 앱 내부 스로틀링 세 항목이 이 문서와 다르다.

- argon2 동시 실행 수는 2가 아니라 코어 수다(`crates/infrastructure/src/auth.rs`의 `Semaphore::new(parallelism)`). auth/5에서 코어 수로 정했고, t3.medium(2 vCPU)에서는 결과적으로 2다.
- 요청 타임아웃과 본문 64KB 제한은 아직 없다. 본문은 axum `Json` 추출기 기본값(2MB)만 걸린다. #27에서 넣는다.
