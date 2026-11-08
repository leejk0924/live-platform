# 미디어 PoC 실기기 확인 (#3)

> 이 문서의 PoC 페이지와 `compose.poc.yml`은 #7에서 MediaMTX 송출 인증을 JWT로 바꾸면서 지웠다. 결과는 ADR Follow-up에 남아 있다. 다시 보려면 `git show 40c6e29:web/public/poc/publish.html`처럼 #19 머지 커밋에서 꺼낸다.

PC 한 대에 스택을 띄우고, 같은 Wi-Fi의 휴대폰에서 송출·시청을 확인한다.
결과는 [`docs/adr/media/1-mediamtx-whip-llhls.md`](../adr/media/1-mediamtx-whip-llhls.md)의 Follow-up 표에 적는다.

| 파일 | 역할 |
|---|---|
| `web/public/poc/publish.html` | WHIP 송출. 화면+탭 소리+마이크 믹싱 또는 카메라+마이크. 송출 측 720p30 / 2.5Mbps 제한과 `getStats` 표시 |
| `web/public/poc/play.html` | 브라우저 내장 LL-HLS 재생과 WHEP 재생. 재생 상태와 `webkitAudioDecodedByteCount` 표시 |
| `compose.poc.yml` | `poc` 경로에만 송출 권한을 여는 오버레이. #7(JWT)에서 사라진다 |

## 1. 띄우기

PC의 LAN IP를 확인한다 (Windows: `ipconfig`의 Wi-Fi IPv4). 아래는 `192.168.0.7` 예시다.

`.env`에 `POC_PUBLISH_PASS`를 넣는다.

```bash
echo "POC_PUBLISH_PASS=$(openssl rand -hex 12)" >> .env
```

LAN IP를 도메인 자리에 넣어 운영 오버레이와 PoC 오버레이를 함께 띄운다. Caddy는 IP에 내부 CA 인증서를 발급한다.

```bash
DOMAIN=192.168.0.7 PUBLIC_IP=192.168.0.7 \
  docker compose -f compose.yml -f compose.prod.yml -f compose.poc.yml up -d --build --wait
```

Windows 방화벽이 묻으면 Docker의 443/tcp, 8189/udp, 8189/tcp 수신을 허용한다.

끝나면 PoC 오버레이 없이 다시 띄운다.

```bash
bash deploy/deploy.sh local
```

## 2. 휴대폰에서 인증서 신뢰

내부 CA라서 휴대폰은 경고를 띄운다. iOS는 경고를 넘겨도 카메라 권한이 막힐 수 있으니 루트 인증서를 설치한다.

```bash
MSYS_NO_PATHCONV=1 docker compose cp caddy:/data/caddy/pki/authorities/local/root.crt ./caddy-root.crt
```

- iOS: 파일을 AirDrop이나 메일로 보내 프로필 설치 → 설정 > 일반 > 정보 > 인증서 신뢰 설정에서 켠다.
- Android: 설정 > 보안 > 인증서 설치 > CA 인증서. 또는 경고 화면에서 계속 진행한다.
- 확인이 끝나면 프로필과 `caddy-root.crt`를 지운다. 저장소에 커밋하지 않는다.

## 3. 확인 항목

송출: `https://192.168.0.7/poc/publish.html`, 사용자 `poc`, 비밀번호는 `.env`의 `POC_PUBLISH_PASS`.
시청: `https://192.168.0.7/poc/play.html`. 내장 HLS가 없는 브라우저는 페이지의 MediaMTX hls.js 플레이어 링크를 쓴다.

| # | 송출 | 시청 | 통과 기준 |
|---|---|---|---|
| 1 | PC Chrome, 화면(탭 선택, "탭 오디오 공유" 체크) | 휴대폰 WHEP | 화면이 보이고 탭 소리와 마이크가 함께 들린다. 송출 통계 `audioTracksMixed: 2` |
| 2 | Android Chrome, 카메라 | PC LL-HLS(hls.js 링크), PC WHEP | 영상과 소리 |
| 3 | iOS Safari, 카메라 | PC LL-HLS(hls.js 링크), PC WHEP | 영상과 소리 |
| 4 | 아무 송출 | **iOS Safari LL-HLS (내장)** | "소리 켜기" 후 소리가 들리고 `webkitAudioDecodedByteCount`가 계속 증가한다. 실패면 iOS는 WHEP fallback |
| 5 | 아무 송출 | iOS Safari WHEP | 영상과 소리 (4번 실패 시 fallback 경로) |
| 6 | PC Chrome 화면(1080p 이상 모니터) | - | 송출 통계 `sent.video`가 높이 ≤ 720, fps ≤ 30, kbps ≤ 2500 근처를 유지 |

각 항목마다 기기·OS·브라우저 버전과 통계 블록의 핵심 값(해상도, fps, kbps, 오디오 바이트 증가 여부)을 적는다.
