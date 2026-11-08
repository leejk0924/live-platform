---
issue: 14
area: common
summary: 보안 헤더와 slowloris 방어는 Caddy 한 곳에, 감사 로그는 target audit 이벤트로 api 어댑터에, 커밋 전 gitleaks는 stdin 모드 git hook으로 둔다
status: confirmed
date: 2026-11-09
paths: [deploy/caddy/Caddyfile, docker/caddy.Dockerfile, crates/api/src/audit.rs, crates/api/src/error.rs, .githooks/pre-commit, .github/dependabot.yml, SECURITY.md, deploy/check-hardening.sh]
related: [5, 13, 15, 27]
---

# 보안 하드닝: 헤더, 감사 로그, 공급망

## Background

Threat Model(S1~S10)의 횡단 항목 중 보안 헤더, Caddy slowloris 방어, 감사 로그, npm audit, gitleaks pre-commit, Dependabot, 위협↔코드 연결 문서가 비어 있었다. 요청 10초·본문 64KB(#27), cargo-deny와 gitleaks CI, non-root·read-only·digest 고정은 이미 있어 확인만 했다. public 전환(#15)의 전제다.

## Decision and reasons

| 결정 | 이유 |
|---|---|
| 보안 헤더는 Caddy 사이트 블록 하나에서 모든 응답(SPA, `/api`, `/media`)에 붙인다 | 모든 요청이 지나는 유일한 지점이라 새 경로도 빠지지 않는다(deny by default). SPA 정적 파일은 API를 거치지 않는다 |
| CSP는 `'self'` 기준에 `worker-src`와 `media-src`만 `blob:` 허용. `unsafe-inline`·`unsafe-eval`·nonce 없음 | hls.js가 blob: Worker로 demux하고 blob: MediaSource URL로 재생한다. Vite 빌드는 인라인 스크립트가 없다. 헤드리스 Chrome에서 앱 렌더링, blob Worker, MSE는 동작하고 eval·인라인 스크립트는 막히는 것을 확인 |
| HSTS 값은 `HSTS_MAX_AGE` 환경 변수. 기본과 운영은 1년, 로컬 `compose.yml`은 0 | 로컬은 Caddy 내부 CA 경고를 손으로 넘겨야 하는데 HSTS 호스트에서는 브라우저가 넘기지 못하게 한다. localhost HSTS는 같은 호스트의 Vite 개발 서버도 https로 바꾼다 |
| Caddy 전역 `servers`에 `read_header 10s`, `read_body 30s`, `idle 2m`, 헤더 32KB. write 타임아웃은 두지 않는다 | 헤더를 조금씩 보내는 연결을 10초에 끊는다. write 타임아웃은 LL-HLS 블로킹 재요청과 SSE(#11)를 끊는다 |
| `:80` 리다이렉트를 `http://` 사이트로 직접 적는다(308) | 자동 HTTPS가 만드는 리다이렉트 서버는 전역 타임아웃을 받지 않는다. `:80`에 반쯤 보낸 헤더가 30초 넘게 열려 있는 것을 실측으로 발견했다. ACME HTTP 챌린지는 HTTP 포트의 모든 서버가 처리한다 |
| Caddy 런타임 이미지를 distroless static으로 바꾸고 healthcheck용 정적 busybox만 `wget`으로 넣는다 | 외부에 노출된 컨테이너에서 셸과 패키지 관리자를 없앤다. api는 이미 distroless, mediamtx는 scratch 기반이다. postgres는 공식 이미지를 그대로 둔다 |
| 감사 로그는 `tracing` 이벤트에 `target: "audit"`과 닫힌 이름 집합(`AuditEvent`)의 `event` 필드. 요청 span에 `trace_id`, `client`, `user` | 별도 저장소 없이 JSON 로그에서 target으로 걸러 낸다. 이름을 enum과 매크로 한 곳에 두어 오타가 날 수 없다 |
| 403은 `trace_id` 미들웨어 한 곳에서, 로그인 실패(`INVALID_CREDENTIALS`, `RATE_LIMITED`)는 로그인 핸들러에서, kick은 api의 세 호출 지점(owner 종료, hook, reaper)에서 `by`와 함께 남긴다 | 403을 미들웨어에서 잡으면 앞으로 생길 라우트도 자동으로 남는다. kick은 누가 시켰는지가 api 계층에서만 보인다 |
| 로그인 실패에는 사용자명 형식을 통과한 값만 남기고, 아니면 표지만 남긴다. 비밀번호는 남기지 않는다 (리뷰 F5) | 스터핑 대상 계정을 추적하려면 계정이 필요하다. 사용자명 칸에 비밀번호를 잘못 넣은 경우 대부분은 형식(소문자·숫자·`_`·`-`, 3~20자)에서 걸러진다 |
| `publisher_kicked`는 "그 방송 경로에 끊기 요청을 보냈다"는 뜻이다 (리뷰 F2) | MediaMTX kick은 세션이 0개여도 성공한다. 끊은 세션 수를 알려면 `StreamTerminator` port 계약을 바꿔야 하는데, 감사 목적에는 누가 언제 끊기를 요청했는지로 충분하다 |
| reaper가 유예가 끝난 방송(`expired`)에 보내는 kick은 실패만 남기고 성공은 남기지 않는다 (리뷰 F4 거부) | `expired`는 송출이 이미 끊겨 INTERRUPTED가 된 방송이라 kick은 안전장치다(`crates/application/src/reaper.rs:92`). 성공을 남기면 실제 끊김 없는 줄만 늘어난다. 활성이 아닌 방송의 송출(`not_active`)은 실제 송출자가 있어 남긴다 |
| 커밋 전 gitleaks는 `.githooks/pre-commit`(`core.hooksPath`)이 staged diff를 gitleaks `stdin`에 넘긴다. 로컬 gitleaks가 없으면 digest 고정 이미지를 docker로 돌리고, 둘 다 없으면 커밋을 막는다 | 저장소를 컨테이너에 마운트하지 않으므로 worktree(`.git` 파일이 Windows 경로를 가리킴)와 소유자 검사 문제가 없다. 검사할 수 없을 때 통과시키지 않는다 |
| npm audit은 CI `web` job에서 high 이상 실패. Dependabot은 cargo, npm, Actions, Dockerfile, compose를 주마다, 앞의 셋은 묶어서 | 낮은 등급까지 막으면 고칠 수 없는 경고로 CI가 멈춘다. 묶음은 PR 수를 줄인다 |
| `SECURITY.md`에 S1~S10별 대응 코드와 테스트, 비어 있는 곳을 적고 `deploy/check-hardening.sh`로 운영 스택을 다시 확인할 수 있게 한다 | 위협 모델은 Obsidian(개인 노트)에만 있었다. public 저장소의 독자가 같은 연결을 볼 수 있어야 한다 |

## Rejected alternatives

- 헤더를 axum 미들웨어로: SPA 정적 파일과 MediaMTX 응답은 API를 거치지 않는다.
- nonce 기반 CSP: 정적 SPA라 요청마다 nonce를 넣을 서버 렌더링이 없고, 인라인 스크립트도 없다.
- HSTS를 로컬에도 1년: 로컬 실행 절차(내부 CA 수동 허용)가 막힌다.
- 감사 로그 port를 application에 두기: 소비자가 로그 하나뿐인데 port, adapter, 주입이 생긴다. 저장소나 외부 전송이 필요해지면 그때 만든다.
- kick 감사를 MediaMTX adapter(`kick`) 한 곳에: 누가 시켰는지 모르고, adapter 테스트에 가짜 HTTP 서버가 필요하다.
- pre-commit 프레임워크(`.pre-commit-config.yaml`): Python과 Go(또는 docker 마운트)가 필요하고, worktree에서 저장소 마운트가 깨진다.
- 결제 감사: 결제 기능이 없다. #13에서 `AuditEvent`에 추가한다.

## Cautions when touching this

- 새 외부 출처(CDN, 폰트, 분석, Toss 위젯)를 쓰면 CSP에 그 출처를 명시적으로 더해야 한다. 안 그러면 브라우저 콘솔에만 오류가 난다.
- 오래 열리는 응답에 Caddy write 타임아웃을 걸지 않는다.
- `http://` 사이트 블록을 지우면 `:80`이 다시 타임아웃 없는 자동 리다이렉트 서버가 된다. `deploy/check-hardening.sh`가 `:80`으로 slowloris를 확인한다.
- 403이 아닌 거부(401 등)는 감사 대상이 아니다. 새 감사 이벤트는 `AuditEvent`에 추가한다. 감사 이벤트 안의 `event` 필드 이름은 예약이다(hook 로그는 `hook`으로 바꿨다).
- `client`는 XFF 마지막 항목이다. 로컬 compose에서는 Docker 게이트웨이 IP로 찍힌다.
- caddy 이미지에 셸이 없다. 디버깅은 `docker compose exec caddy caddy environ` 같은 caddy 하위 명령으로 한다.
- (unverified) 운영 도메인에서 HSTS 1년과 Let's Encrypt 발급을 함께 확인하지 않았다(`compose.prod.yml` 렌더링 결과만 확인).
