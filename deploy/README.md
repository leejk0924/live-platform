# 배포

서버 1대에 docker compose로 caddy, api, mediamtx, postgres를 올린다. 로컬과 운영은 `compose.prod.yml` 오버레이와 `.env` 값만 다르다.
설계 근거는 [`docs/adr/infra/1-aws-single-ec2-compose.md`](../docs/adr/infra/1-aws-single-ec2-compose.md).

> 검증은 로컬에서만 한다. 운영 절차(3~6절)는 문서로만 두며 실제 서버에서 확인하지 않았다.

## 1. 구성

| 서비스 | 외부 포트 | 역할 |
|---|---|---|
| caddy | 80, 443/tcp, 443/udp | HTTPS 종료, `/api/*` → api, `/media/hls/*`·`/media/webrtc/*` → mediamtx, 나머지는 SPA |
| api | 없음 | Rust API |
| mediamtx | 8189/udp, 8189/tcp | WebRTC 미디어. HLS(8888)와 WHIP/WHEP 시그널링(8889)은 Caddy를 거쳐 공개, Control API(9997)는 내부만 |
| postgres | 없음 | 데이터 |

| 파일 | 내용 |
|---|---|
| `compose.yml` | 공통 정의. 리소스 제한, non-root, read-only rootfs, healthcheck |
| `compose.prod.yml` | 운영에서 바뀌는 값: `SITE_ADDRESS`(도메인), `MTX_WEBRTCADDITIONALHOSTS`(공인 IP) |
| `deploy/caddy/Caddyfile` | 라우팅, 보안 헤더 |
| `deploy/mediamtx/mediamtx.yml` | 미디어 서버 설정 |
| `deploy/deploy.sh` | `up --build --wait` 후 `/api/healthz` 확인 |

## 2. 로컬 실행

필요: Docker Engine과 Compose v2 (`docker compose version`).

```bash
cp .env.example .env
# 로컬은 값을 그대로 둬도 동작한다. 비밀번호만 무작위로 바꾸려면 (macOS sed도 동작):
sed -i.bak "s/^POSTGRES_PASSWORD=.*/POSTGRES_PASSWORD=$(openssl rand -hex 24)/" .env && rm .env.bak
bash deploy/deploy.sh local
```

`ok: https://localhost is up (<커밋>)`이 나오면 성공이다. 브라우저에서 `https://localhost`를 열 때 인증서 경고가 나오는 것은 Caddy 내부 CA를 쓰기 때문이다.

## 3. 운영 서버 요건

- Linux 서버 1대, 2 vCPU / 4GB 이상 (컨테이너 메모리 상한 합 약 2.4GB)
- 고정 공인 IP
- 인바운드: 80/tcp, 443/tcp, 8189/udp, 8189/tcp. HTTP/3를 쓰려면 443/udp도 연다. 그 외는 닫는다 (Postgres, MediaMTX API는 열지 않는다)
- Docker Engine + Compose v2, git
- 이미지를 서버에서 빌드한다. Rust 릴리스 빌드는 메모리를 많이 써서 4GB 서버에서는 swap 2GB를 권한다

```bash
sudo fallocate -l 2G /swapfile && sudo chmod 600 /swapfile
sudo mkswap /swapfile && sudo swapon /swapfile
echo '/swapfile none swap sw 0 0' | sudo tee -a /etc/fstab
```

## 4. 도메인 (DuckDNS)

1. duckdns.org에서 서브도메인을 만들고 서버 공인 IP를 넣는다.
2. 서버 밖에서 `nslookup <이름>.duckdns.org`가 그 IP를 돌려주는지 확인한다.
3. 인증서는 Caddy가 시작할 때 Let's Encrypt에서 받는다. 80과 443이 열려 있어야 한다. `deploy.sh prod`는 발급을 기다리며 healthz를 최대 1분 재시도한다.

## 5. 첫 배포

```bash
git clone https://github.com/leejk0924/live-platform.git
cd live-platform
cp .env.example .env
chmod 600 .env
```

`.env`의 모든 `change-me`를 바꾼다. 남아 있으면 `deploy.sh prod`가 멈춘다.

| 키 | 값 |
|---|---|
| `POSTGRES_PASSWORD` | `openssl rand -hex 24` 결과 |
| `ACME_EMAIL` | 인증서 만료 알림을 받을 이메일 |
| `DOMAIN` | `<이름>.duckdns.org` |
| `PUBLIC_IP` | 서버 공인 IP (WebRTC ICE 후보로 쓰인다) |
| `DEMO_LOGIN_ENABLED` | 데모 계정 로그인을 열려면 `true`, 아니면 `false` |

```bash
bash deploy/deploy.sh prod
```

## 6. 운영

| 작업 | 명령 |
|---|---|
| 업데이트 | `git pull --ff-only && bash deploy/deploy.sh prod` |
| 롤백 | `git checkout <이전 커밋> && bash deploy/deploy.sh prod` (끝나면 `git switch main`) |
| 상태 | `docker compose -f compose.yml -f compose.prod.yml ps` |
| 로그 | `docker compose -f compose.yml -f compose.prod.yml logs -f --tail 100 <서비스>` |
| 리소스 사용량 | `docker stats --no-stream` |
| DB 백업 | `docker compose -f compose.yml -f compose.prod.yml exec -T postgres sh -c 'pg_dump -U "$POSTGRES_USER" "$POSTGRES_DB"' > ~/backup-$(date +%F).sql` (저장소 밖에 쓴다) |
| 오래된 이미지 정리 | `docker image prune -f` |

로그는 컨테이너마다 10MB × 3개로 회전한다.

## 7. 문제 해결

| 증상 | 확인할 것 |
|---|---|
| 인증서가 발급되지 않는다 | DNS가 서버 IP를 가리키는지, 80·443이 열려 있는지, `caddy` 로그의 ACME 오류. Let's Encrypt는 실패가 반복되면 한동안 막으므로 원인을 고친 뒤 재시도한다 |
| 페이지는 열리는데 송출·시청이 연결되지 않는다 | `PUBLIC_IP`가 맞는지, 8189/udp·tcp가 열려 있는지, 브라우저 `chrome://webrtc-internals`의 ICE 후보 |
| `deploy.sh`가 `--wait`에서 멈춘다 | `docker compose ps`에서 unhealthy인 서비스의 로그 |
| 빌드 중 서버가 멈춘다 | 메모리 부족. swap(3절) 확인 |

## 8. AWS 관련 (미적용)

ADR은 AWS EC2를 운영 서버로 정했지만, 지금은 AWS에서 배포·검증하지 않는다. 쓰게 되면 ADR의 결정을 따른다: t3.medium(서울), CPU 크레딧 `standard`, Elastic IP, IMDSv2 강제, IAM Role(Access Key 없음), SSH 대신 SSM, 보안 그룹은 3절의 포트만.
