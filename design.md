---
version: "1.0"
name: "live-platform 다크 스튜디오"
description: "실시간 방송 플랫폼 웹앱(React SPA)의 디자인 기준. 다크 기본, 라이트 지원. 성능 하드웨어 계열의 검정·빨강·전기 파랑 톤을 방송 화면에 맞게 옮긴다."
themes: [dark, light]   # dark가 기본. prefers-color-scheme: light면 light
colors:
  dark:
    bg: "#0E0E10"            # 페이지 배경. 순수 검정은 쓰지 않는다
    surface-1: "#18181B"     # 카드, 사이드바, 상단 바
    surface-2: "#232327"     # 입력, hover, 카드 안의 칸
    border: "#2E2E33"
    text: "#F4F4F5"
    text-muted: "#A1A1AA"
    primary: "#ED1C24"       # 브랜드 빨강. 점, 테두리, 밑줄, 글로우 같은 장식에만
    primary-fill: "#C8151B"  # 흰 글자를 얹는 면(주 버튼, LIVE 배지)
    primary-text: "#FF4D52"  # 어두운 면 위의 빨간 글자
    on-primary: "#FFFFFF"
    accent: "#4FB3E0"        # 링크, 포커스 링, 선택된 탭, 저지연 표시
    success: "#34C77B"
    warning: "#F2A93B"       # INTERRUPTED, 슬로우 모드
    error: "#FF6B6B"
  light:
    bg: "#FAFAFA"
    surface-1: "#FFFFFF"
    surface-2: "#F4F4F5"
    border: "#E4E4E7"
    text: "#18181B"
    text-muted: "#52525B"
    primary: "#ED1C24"
    primary-fill: "#C8151B"
    primary-text: "#C8151B"
    on-primary: "#FFFFFF"
    accent: "#0A6FA0"
    success: "#15803D"
    warning: "#A15C07"
    error: "#B91C1C"
status:                      # 방송 상태 배지. 색만으로 구분하지 않고 글자를 항상 함께 쓴다
  LIVE:        { fill: "{colors.primary-fill}", text: "{colors.on-primary}", label: "LIVE", dot: pulse }
  INTERRUPTED: { fill: "transparent", border: "{colors.warning}", text: "{colors.warning}", label: "연결 끊김" }
  READY:       { fill: "transparent", border: "{colors.accent}", text: "{colors.accent}", label: "준비 중" }
  ENDED:       { fill: "{colors.surface-2}", text: "{colors.text-muted}", label: "종료" }
typography:
  font-sans: "Pretendard, 'Apple SD Gothic Neo', 'Malgun Gothic', system-ui, -apple-system, 'Segoe UI', Roboto, sans-serif"
  font-mono: "ui-monospace, 'JetBrains Mono', 'Cascadia Mono', Consolas, monospace"
  h1:      { size: 1.75rem, weight: 700, line: 1.25 }
  h2:      { size: 1.25rem, weight: 700, line: 1.3 }
  title:   { size: 1rem, weight: 600, line: 1.4 }     # 카드 제목, 사이드바 제목
  body-md: { size: 0.9375rem, weight: 400, line: 1.6 }
  small:   { size: 0.8125rem, weight: 500, line: 1.4 }
  mono-sm: { size: 0.8125rem, weight: 500 }            # kbps, 해상도, 금액, 시각
spacing: { unit: 8px, xs: 4px, sm: 8px, md: 16px, lg: 24px, xl: 32px, xxl: 48px }
rounded: { sm: 4px, md: 8px, lg: 12px, pill: 999px }
layout:
  max-width: 1440px
  gutter-desktop: 24px
  gutter-mobile: 16px
  sidebar-width: 360px
  breakpoints: { mobile: 0, tablet: 768px, desktop: 1100px }
motion:
  fast: 150ms
  base: 220ms
  enter: 360ms
  easing: "cubic-bezier(0.2, 0, 0, 1)"
z-index: { base: 0, sticky-nav: 100, overlay: 200, modal: 300, toast: 500 }
components:
  button-primary:   { bg: "{colors.primary-fill}", text: "{colors.on-primary}", radius: "{rounded.md}", padding: "10px 16px", weight: 600, min-height: 44px }
  button-secondary: { bg: "transparent", border: "1px solid {colors.border}", text: "{colors.text}", radius: "{rounded.md}", min-height: 44px }
  button-danger:    { bg: "transparent", border: "1px solid {colors.error}", text: "{colors.error}" }
  input:            { bg: "{colors.surface-2}", border: "1px solid {colors.border}", radius: "{rounded.md}", focus-ring: "2px {colors.accent}, offset 2px" }
  card:             { bg: "{colors.surface-1}", border: "1px solid {colors.border}", radius: "{rounded.lg}" }
---

## Overview

방송을 고르고, 보고, 대화하고, 후원하는 화면이다. 주인공은 영상이고 UI는 그 둘레에서 조용히 정보를 준다. 성능 하드웨어 계열의 톤(어두운 면, 빨간 신호, 전기 파랑 강조, 기술적인 숫자 표기)을 빌리되 랜딩 페이지의 연출(히어로, 지그재그 섹션, 큰 글로우)은 쓰지 않는다.

- 밀도 7/10: 목록과 댓글은 촘촘하게, 플레이어 둘레는 여유 있게.
- 변화 4/10: 화면마다 같은 카드·버튼·배지를 쓴다.
- 모션 3/10: 상태가 바뀌었다는 신호로만 쓴다(LIVE 점, 새 후원, 새 댓글).
- 빨강은 "지금 방송 중"과 "주요 행동"의 신호다. 장식으로 넓게 깔지 않는다.
- 특정 서비스의 로고, 이름, 고유 색 조합을 따라 하지 않는다.

## Colors

토큰은 `:root`의 CSS 변수(`--color-bg` 등)로 두고, 라이트는 `@media (prefers-color-scheme: light)`에서 같은 이름을 다시 정의한다. 컴포넌트는 hex를 직접 쓰지 않고 변수만 쓴다.

- 깊이는 그림자가 아니라 면의 밝기 단계(`bg` → `surface-1` → `surface-2`)와 1px `border`로 만든다. 다크에서 그림자는 보이지 않는다.
- 빨강은 세 가지로 나눠 쓴다. 흰 글자를 얹는 면은 `primary-fill`(흰 글자 대비 5.9:1), 어두운 면 위 글자는 `primary-text`(5.4:1), 글자가 없는 장식은 `primary`. `primary` 위에 흰 글자를 얹으면 4.4:1이라 본문 크기 기준(4.5:1)에 못 미친다.
- 에러는 `error`이고 빨간 브랜드색과 섞지 않는다. 삭제 버튼은 테두리형 `button-danger`.
- 형광색(`#00FF00` 같은)은 쓰지 않는다.

확인한 대비(WCAG, 배경 대비 글자):

| 쌍 | 다크 | 라이트 |
|---|---|---|
| `text` / 배경 | 17.5 | 17.0 |
| `text-muted` / `surface-1` | 6.9 | 7.7 |
| `accent` / 배경 | 8.1 | 5.5 |
| `on-primary` / `primary-fill` | 5.9 | 5.9 |
| `success` / `surface-1` | 8.1 | 5.0 |
| `warning` / `surface-1` | 8.9 | 5.2 |
| `error` / `surface-1` | 6.4 | 6.5 |

## Typography

- 한글이 먼저 보이는 화면이라 한글 글꼴을 앞에 둔다. Pretendard는 설치된 기기에서만 쓰이고, 없으면 Apple SD Gothic Neo / 맑은 고딕으로 떨어진다. 글꼴 파일을 저장소에 넣어 직접 서빙하기로 하면 OFL 라이선스를 확인하고 README Credits에 추가한다. 외부 글꼴 CDN은 CSP(`style-src 'self'`, `font-src` 기본 `'self'`)에 막힌다.
- 숫자(금액, kbps, 해상도, 시청 시각, 후원 합계)는 `font-mono` 또는 `font-variant-numeric: tabular-nums`로 자리가 흔들리지 않게 한다.
- 한 줄 최대 72자. 방송 제목은 카드에서 두 줄까지 보이고 말줄임.

## Layout

공통: 상단 바(높이 56px, `surface-1`, 아래 1px `border`, 스크롤해도 붙어 있음). 왼쪽 로고와 `다시보기`, 오른쪽 `방송하기`(로그인 시, `button-primary`)와 계정 메뉴(로그인 / 회원가입 / 로그아웃).

| 화면 | 데스크톱(≥1100px) | 태블릿·휴대폰(<1100px) |
|---|---|---|
| 홈 `/` | `방송 중` 카드 그리드(`repeat(auto-fill, minmax(280px, 1fr))`), 그 아래 `지난·예정 방송` 같은 그리드 | 2열(≥768px), 1열(<768px) |
| 방송 상세 `/broadcasts/:id` | 왼쪽 플레이어 + 방송 정보, 오른쪽 360px 사이드바(위 후원, 아래 댓글이 남은 높이를 채움) | 플레이어 → 방송 정보 → 후원 → 댓글 순으로 세로 |
| 소유자 상세 | 플레이어 자리에 송출 패널(미리보기, 상태 줄, 시작·종료·카메라 전환), 사이드바 동일 | 송출 패널이 맨 위, 버튼은 줄을 채움 |
| 방송 만들기·수정 | 가운데 카드 하나(최대 560px), 라벨 위·입력 아래 | 같은 카드가 폭을 채움 |
| 로그인·가입 | 가운데 카드(최대 400px). 데모 계정 버튼은 구분선 아래 별도 칸 | 같음 |
| 다시보기 목록 | 홈과 같은 카드 그리드, 카드에 길이 표시 | 홈과 같음 |
| 다시보기 상세 | 플레이어 + 정보, 사이드바 없음(최대 960px) | 세로 |
| 결제 성공·실패 | 가운데 카드에 결과, 금액, `방송으로 돌아가기` | 같음 |

- 썸네일: 서버가 만들지 않는다. 카드 위쪽 16:9 칸에 상태별 CSS 그라데이션과 방송인 이니셜을 그린다. 외부 이미지(`img-src 'self'`)는 쓰지 않는다.
- 휴대폰 360~412px에서 가로 스크롤 0, 터치 대상 44px 이상(#9에서 확인한 기준 유지).
- 플레이어 영역은 16:9를 지키고 세로 영상은 가운데 정렬, 좌우는 `bg`.

## Components

- **주 버튼**: `button-primary`. hover는 밝기 +6%, active는 `translateY(1px)`. 글로우 없음. 한 화면에 주 버튼은 하나(`방송 시작`, `후원하기`, `등록` 등).
- **보조 버튼**: 테두리형. `소리 켜기`, `다시 시도`, `취소`, `수정`.
- **위험 버튼**: `방송 종료`, `삭제`. 테두리 `error`. 삭제는 확인 단계를 둔다(지금 동작 유지).
- **상태 배지**: front matter `status`. LIVE는 빨간 면에 흰 글자, 앞에 2초 주기로 깜빡이는 점. INTERRUPTED·READY는 테두리형, ENDED는 회색 면. 글자 없이 색만 쓰지 않는다.
- **방송 카드**: 썸네일 칸(왼쪽 위 상태 배지), 아래 제목(`title`, 두 줄), 방송인·시작 시각(`small`, `text-muted`). 카드 전체가 링크. hover는 `border`가 `accent`로, 썸네일만 `scale(1.02)`.
- **플레이어**: 영상 아래 한 줄 컨트롤 바(`surface-1`). 왼쪽 `소리 켜기`, 가운데 재생 중인 방식(`LL-HLS로 재생 중` / `WebRTC로 재생 중`, `small`), 오른쪽 `저지연(WebRTC)` 토글을 스위치 모양으로. 종료·끊김 안내는 영상 자리에 같은 크기의 칸으로 보여 화면이 튀지 않게 한다.
- **송출 상태 줄**: `.publish-state`. `송출 중 · 2,345 kbps · 1280x720`을 `mono-sm`로, 앞에 LIVE 점. 오류는 그 아래 `error` 글자.
- **댓글**: 한 줄에 닉네임(`title` 굵기, 닉네임마다 고정 색은 쓰지 않고 `accent`), 본문, 시각(`small`, `text-muted`). 새 댓글이 오면 목록 맨 아래에 붙고, 사용자가 위로 스크롤해 있으면 따라 내리지 않고 `새 댓글` 칩을 띄운다. 입력 칸은 사이드바 맨 아래 고정, 500자 카운터, 슬로우 모드면 입력 위에 `warning` 안내.
- **후원**: 사이드바 위 칸. 머리글에 합계(`mono-sm`)와 건수. 금액 프리셋은 알약 모양 칩, 선택된 칩은 `primary-fill`. 새 후원 알림 `.donation-fresh`는 `primary` 왼쪽 테두리 + `surface-2` 면으로 360ms 들어오고 6초 뒤 사라진다.
- **입력**: 라벨은 위, 오류 글자는 아래 `error`. 떠오르는 라벨은 쓰지 않는다.
- **빈 상태**: 아이콘 하나 + 한 문장. 행동 버튼은 빈 상태 안에 두지 않는다. 방송을 시작하는 길은 상단 바의 `방송하기`와 홈의 `방송 만들기` 버튼이 맡는다.
- **로딩**: 카드·댓글·후원 목록은 자리 크기 그대로의 스켈레톤(은은한 shimmer). 원형 스피너는 플레이어 버퍼링에만.
- **아이콘**: Lucide를 npm 패키지로 번들에 넣거나 SVG를 직접 넣는다. CDN은 쓰지 않는다. 이모지는 UI에 쓰지 않는다.

## Motion

- `transform`과 `opacity`만 움직인다. 레이아웃을 바꾸는 속성은 애니메이션하지 않는다.
- 들어오기: fade + `translateY(8px → 0)`, `motion.enter`. 목록은 처음 그릴 때만 항목당 40ms 차이(최대 8개).
- hover: 색·테두리 변화 `motion.fast`.
- 페이지 전환: fade `motion.base`.
- `prefers-reduced-motion: reduce`면 LIVE 점 깜빡임, shimmer, 들어오기를 끄고 즉시 바꾼다.

## Constraints

- **CSP**: `style-src 'self'`라 인라인 `style` 속성과 외부 CSS·글꼴을 쓰지 않는다. 스타일은 `web/src` 안의 CSS 파일로만(CSS 프레임워크 추가 없음). 동적인 값이 꼭 필요하면 클래스나 `data-*` 속성으로 바꾼다.
- **E2E 핸들 유지**: `.claude/skills/verify-live-platform/SKILL.md`의 handles 표(aria-label, `.publish-state`, `.donation-*`, `section.player[aria-label=…]`, 버튼 이름)를 바꾸지 않는다. 바꿔야 하면 같은 PR에서 여정도 고친다.
- **문구 유지**: 버튼·안내 문구는 지금 문장을 유지한다(여정이 이름으로 찾는다). 새 문구를 쓸 때는 과장된 마케팅 표현을 쓰지 않는다.
- **접근성**: 본문 대비 4.5:1 이상, 포커스 링을 숨기지 않는다, 상태는 색과 글자를 함께.

## Do's and Don'ts

- Do: 영상이 가장 크게, LIVE는 한눈에, 숫자는 고정폭.
- Do: 같은 역할에는 같은 컴포넌트.
- Don't: 순수 검정 `#000000`, 형광색, 큰 글로우, 그림자로 깊이 만들기.
- Don't: 히어로 배너, 지그재그 섹션, 랜딩 페이지식 큰 여백.
- Don't: `100vh` 고정 높이(휴대폰 주소창). 필요하면 `100dvh`.
- Don't: 외부 이미지·글꼴·아이콘 CDN, 인라인 스타일.
- Don't: lorem ipsum. 예시는 실제 화면 문구로.
