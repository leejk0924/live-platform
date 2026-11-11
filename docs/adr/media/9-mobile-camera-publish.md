---
issue: 9
area: media
summary: 모바일은 카메라 송출을 같은 WHIP 경로로 하고, 전면·후면 전환은 replaceTrack으로, 화면 꺼짐·백그라운드는 송출을 닫아 INTERRUPTED로 넘긴 뒤 30초 안에 돌아오면 자동으로 다시 송출한다
status: confirmed
date: 2026-11-09
paths: [web/src/broadcasts/publish/capture.ts, web/src/broadcasts/publish/whip.ts, web/src/broadcasts/publish/phase.ts, web/src/broadcasts/publish/Publisher.tsx, web/src/broadcasts/platform.ts, web/src/index.css]
related: [1, 3, 7, 8, 10, 12, 14]
---

# 모바일 카메라 송출

## Background

#7의 송출 화면은 PC 화면 공유(`getDisplayMedia`)만 지원했다. Android Chrome과 iOS Safari에는 `getDisplayMedia`가 없어서 휴대폰에서는 송출을 시작할 수 없었다. #3 PoC에서 iPhone Safari 카메라 → WHIP는 실기기로 통과했고, Android는 확인하지 못했다(media/1). 휴대폰은 화면이 꺼지거나 다른 앱으로 가면 카메라가 멈춘다. 이때 방송 상태를 어떻게 할지 정해야 했다.

## Decision and reasons

| 결정 | 이유 |
|---|---|
| 송출 소스를 `Capture = ScreenCapture \| CameraCapture` 합 타입으로 둔다. 휴대폰이면 카메라만, PC면 `getDisplayMedia`·`getUserMedia` 유무에 따라 화면(기본)과 카메라를 고른다 | WHIP, 상한, 통계 코드는 소스와 무관하게 그대로 쓴다. 휴대폰에는 선택지 없이 카메라만 보인다 |
| 휴대폰 판별은 `userAgentData.mobile`, iOS(iPadOS 포함, #8 `isIos`), Android UA로 한다. `getDisplayMedia`가 없다는 사실로 판별하지 않는다 (리뷰 F2) | 처음에는 "`getDisplayMedia` 없음 = 휴대폰"으로 했다. MDN browser-compat-data에 iOS Safari 27이 `getDisplayMedia`를 지원한다고 나와 있어, iOS 27에서는 기본 소스가 화면이 되고 백그라운드 일시정지도 빠진다 |
| 카메라와 마이크는 `getUserMedia` 한 번으로 받는다. `facingMode: exact`가 안 맞으면(PC 웹캠은 facing이 없다) 느슨한 조건으로, 마이크가 없으면 영상만으로 다시 시도한다 | 권한 창을 한 번만 띄운다. 권한 거부는 다시 시도하지 않는다 |
| 전면·후면 전환은 지금 카메라를 먼저 끄고 다른 쪽을 연 뒤 `RTCRtpSender.replaceTrack`으로 바꾼다. 다른 쪽이 `exact` facing으로 안 열리면 다른 `videoinput` 장치로, 그것도 안 되면 원래 쪽을 다시 열어 방송 화면을 유지한다 | WHIP 세션과 협상을 그대로 둬 시청자가 끊기지 않는다(헤드리스 e2e에서 시청 시간이 계속 늘어남을 확인). iOS는 두 번째 `getUserMedia`가 첫 영상 트랙을 죽인다고 보고됐다([WebKit 179363](https://bugs.webkit.org/show_bug.cgi?id=179363)). `replaceTrack`이 실패하면 보내는 트랙이 이미 끈 카메라라 카메라를 잃은 것으로 처리한다(리뷰 F1) |
| 720 상한(`scaleResolutionDownBy`)은 짧은 변에 건다 | 세로 휴대폰 프레임(720x1280)을 높이 기준으로 줄이면 405x720이 된다 |
| 휴대폰에서 송출 중 페이지가 `hidden`이 되면 송출을 닫는다(WHIP DELETE는 `keepalive`). 서버는 #10 hook으로 INTERRUPTED가 된다. `visible`로 돌아오면 같은 소스·방향으로 자동으로 start를 다시 불러 송출한다. 30초가 지나면 reaper가 ENDED로 끝내고, 돌아왔을 때 마지막으로 본 상태가 ENDED이거나 30초 넘게 떠나 있었으면 카메라를 켜지 않는다(리뷰 F4, F6). 이 판단은 순수 함수 하나로 두고 테스트로 고정한다(리뷰 F3) | 모바일 브라우저는 백그라운드에서 카메라를 멈춘다. 연결을 그대로 두면 시청자는 멈춘 화면을 보고 상태는 LIVE로 남는다. 닫으면 상태가 사실대로 바뀌고, 복귀는 #10의 재연결 규칙(INTERRUPTED에서 start는 토큰 재발급)을 그대로 쓴다 |
| PC(화면·카메라)는 숨겨져도 송출을 유지한다 | 화면 공유자는 공유할 탭으로 계속 옮겨 다닌다. 데스크톱 브라우저는 숨긴 탭에서도 캡처를 유지한다 |
| 카메라 트랙이 `ended`(다른 앱이 카메라를 가져감)면 연결 끊김과 같게 처리한다. stop API는 부르지 않는다. 화면 공유의 `ended`(브라우저 공유 바에서 중지)는 지금처럼 방송 종료다 | 카메라를 잃은 것은 방송을 끝내겠다는 뜻이 아니다. #7의 "끊김은 종료가 아니다"와 같다 |
| 송출 중에는 Screen Wake Lock을 잡는다. 지원하지 않거나 거부되면 무시한다 | 화면이 저절로 꺼져 송출이 멈추는 일을 줄인다. 잠금은 페이지가 숨겨지면 브라우저가 풀어 준다([MDN](https://developer.mozilla.org/en-US/docs/Web/API/Screen_Wake_Lock_API)) |
| 미리보기는 전면 카메라일 때만 좌우 반전한다. 보내는 영상은 반전하지 않는다 | 거울처럼 보이는 편이 촬영하기 쉽고, 시청자는 실제 방향을 본다 |
| 반응형은 영상 `max-height: 70vh`, 터치 기기 버튼 44px 이상, 좁은 화면에서 버튼이 줄을 채우게 한다 | 가로로 든 휴대폰에서도 송출 버튼이 화면 안에 남는다. 360~412px 폭과 가로 회전에서 가로 스크롤이 0임을 확인 |
| CSP와 `Permissions-Policy`는 바꾸지 않는다 | `camera=(self), microphone=(self)`가 이미 있다. 카메라 미리보기는 `srcObject`라 `media-src`와 무관하고 WHIP는 같은 출처다. 헤드리스 Chrome에서 위반 이벤트 0건, `check-hardening.sh local` 통과 |

## Rejected alternatives

- 백그라운드에서도 연결을 그대로 두기: Android에서 카메라가 멈춰도 ICE는 살아 있어 시청자는 멈춘 화면, 상태는 LIVE로 남는다(추론, 실기기 미확인).
- 백그라운드에서 stop API 호출: 알림 확인이나 잠깐의 앱 전환으로 방송이 끝난다. 30초 유예가 무의미해진다.
- 카메라를 `<canvas>`에 그려 고정 해상도로 보내기(회전 대응): 휴대폰 CPU와 배터리 부담이 크고 확인할 기기가 없다. 아래 Cautions의 회전 문제가 실기기에서 크면 다시 본다.
- 모든 소스에서 숨김 시 일시정지: PC 화면 공유가 탭을 옮길 때마다 끊긴다.

## Cautions when touching this

- **휴대폰을 돌리면 송출 해상도가 세로↔가로로 바뀐다.** MediaMTX의 WHIP 응답은 영상 방향 확장(`urn:3gpp:video-orientation`, CVO)을 협상하지 않는다(응답 SDP로 확인). 그래서 브라우저가 픽셀을 돌려 보낸다(추론). 헤드리스에서 송출 중 720x1280 → 1280x720으로 바꾸니 WHEP 시청과 녹화(다시보기 3구간 모두 해당 크기로 재생)는 따라갔지만, 이미 보고 있던 LL-HLS(hls.js) 시청자는 멈추거나 비율이 틀어졌다. 새로 들어온 시청자는 정상이다. 송출 화면에 "방송 중 휴대폰을 돌리지 말라"는 안내를 둔다. 후속 이슈 후보다.
- 복귀 때 다시 송출할지는 마지막으로 본 상태(ENDED)와 떠나 있던 시간(30초 이상) 둘로 정한다. 휴대폰은 백그라운드에서 페이지를 멈춰 5초 새로 고침이 돌지 않으므로 상태만 보면 끝난 방송에 카메라를 켠다(리뷰 F6). 시간 기준은 일시정지한 시각부터라 서버 유예(INTERRUPTED가 된 뒤 30초)보다 몇 초 이르다. 그 사이에 돌아오면 손으로 "방송 시작"을 누르면 이어진다.
- 다시 송출은 진행 중이면 다시 들어오지 않는다. 다시 그리기 전에 복귀 이벤트가 두 번 오면 start가 두 번 불렸다(리뷰 F8, e2e에서 2회 → 1회).
- Android Chrome의 "데스크톱 사이트" 모드는 UA와 `userAgentData.mobile`이 PC로 보여 백그라운드 일시정지가 빠진다(리뷰 F7, 확인 필요). 실기기로 확인하지 않았다(unverified).
- 숨김 판정은 `visibilitychange`다. iOS 권한 창, 알림 센터, 화면 잠금이 `hidden`을 내는지는 실기기로 확인하지 않았다(unverified).
- 복귀 후 자동 재송출은 사용자 제스처 없이 `getUserMedia`를 부른다. WebKit은 `getUserMedia`에 HTTPS를 요구한다([WebKit 블로그](https://webkit.org/blog/7763/a-closer-look-into-webrtc/)). 제스처가 필요하다는 공식 근거는 찾지 못했다(unverified). 실패하면 오류와 "방송 시작" 버튼이 남아 손으로 다시 시작할 수 있다.
- iOS Safari에서 백그라운드 뒤 트랙이 `muted`로 남거나 오디오 트랙이 `ended`가 되는 보고가 있다([WebKit 208516](https://bugs.webkit.org/show_bug.cgi?id=208516), [204681](https://bugs.webkit.org/show_bug.cgi?id=204681)). 이 설계는 복귀 때 트랙을 새로 받으므로 영향이 없을 것으로 본다(unverified).
- 코덱은 #7처럼 H.264를 우선한다. H.264 하드웨어 인코더가 없는 Android 기기는 VP8로 갈 수 있고, 그때 LL-HLS와 녹화가 되는지는 확인하지 않았다(unverified).
- 헤드리스 Chrome의 가짜 카메라 `fake_device_1`은 인코더에 프레임을 넘기지 않는다(처음부터 그 장치로 보내도 `framesSent=0`). 전환 e2e는 이 장치를 목록에서 빼고 `fake_device_2`로 했다.
- 소유자 상세 화면은 #8대로 자기 방송의 Player도 띄운다. 휴대폰 송출자는 올리면서 LL-HLS도 내려받는다. 후속 이슈 후보다.
- 실기기(Android Chrome, iOS Safari) 송출, 전환, 백그라운드, 회전은 확인하지 않았다(unverified). 헤드리스 Chrome의 모바일 에뮬레이션(UA, 뷰포트, 터치, 가짜 카메라, 합성 `visibilitychange`)으로만 확인했다.

## Follow-up (2026-11-11)

실기기 확인. iPhone 15 Pro Safari, LAN 스택(`compose.prod.yml`, `DOMAIN`·`PUBLIC_IP`=PC Wi-Fi IP, Caddy 내부 CA 루트 설치), PC 브라우저로 시청.

- 확인됨: 카메라 영상·마이크 음성 송출, 송출 중 전면·후면 전환(시청 끊김 없음), 백그라운드 30초 안 복귀 시 제스처 없이 자동 재송출, 40초 뒤 ENDED, `방송 종료`, 세로·가로 화면.
- 그래서 위 Cautions의 "복귀 후 자동 재송출은 제스처 없이 `getUserMedia`를 부른다"와 "화면 잠금이 `hidden`을 내는지"는 iOS Safari에서 동작함을 확인했다(iOS 버전은 기록하지 않음).
- Android Chrome 실기기는 기기가 없어 확인하지 않는다. Pixel 7 에뮬레이션 결과로 갈음하고 README 지원 환경에 그렇게 적었다. Android 항목(데스크톱 사이트 모드, VP8 fallback, 백그라운드 때 ICE)은 계속 unverified다.
- 송출 중 회전(CVO 미협상)은 이번에 확인하지 않았다.

## Follow-up (2026-11-12)

#56에서 소유자 상세 화면은 ENDED 전까지 Player 대신 송출 패널만 보인다([common/56](../common/56-ui-design-tokens.md)). 위 Cautions의 "소유자 상세 화면은 자기 방송의 Player도 띄운다"는 더 이상 맞지 않는다.
