---
issue: 56
area: common
summary: 화면 디자인은 design.md 한 문서와 index.css :root 토큰 한 곳에 두고(다크 기본·라이트는 prefers-color-scheme), 빨강은 장식·채움·글자 세 역할로 나누며, 소유자 상세는 플레이어 대신 송출 패널을 보이고, 화면 품질은 layout 여정이 대비·터치·기본 스타일·가로 스크롤로 검사한다
status: confirmed
date: 2026-11-12
paths: [design.md, web/src/index.css, web/src/pages/BroadcastDetail.tsx, web/src/ui/Feed.tsx, web/src/comments/commentTime.ts, .claude/skills/verify-live-platform/scripts/journeys/layout.mjs]
related: [8, 9, 14]
---

# 화면 디자인 토큰과 layout 검사

## Background

#56 이전 SPA는 브라우저 기본 스타일에 가까웠다(파란 밑줄 링크, 기본 버튼, 640px 한 줄 레이아웃). 심사자가 처음 열었을 때 방송 플랫폼으로 보이게 하되 기능과 API는 바꾸지 않는다. 제약은 CSP `style-src 'self'`(#14: 외부 CSS·글꼴 CDN과 인라인 style 금지), verify 여정의 핸들과 기존 UI 문구, 휴대폰 360~412px 가로 스크롤 0·터치 44px(#9)다.

## Decision and reasons

| 결정 | 이유 |
|---|---|
| 디자인 기준은 저장소 루트 `design.md`, 값은 `web/src/index.css`의 `:root` CSS 변수 한 곳 | 화면마다 색·간격을 따로 정하면 어긋난다. 문서는 사람이, 토큰은 코드가 읽는다 |
| 다크 기본, 라이트는 `prefers-color-scheme`만. 전환 버튼 없음 | 방송 화면은 어두운 바탕이 영상을 돋보이게 한다. 전환 버튼은 저장 상태가 생기는데 얻는 것이 적다 |
| 빨강을 `primary`(장식만), `primary-fill`(흰 글자를 얹는 면), `primary-text`(빨간 글자) 셋으로 나눈다. 에러 색은 따로 | 브랜드 빨강 위 흰 글자는 4.5:1에 못 미친다(design.md 대비 표). 같은 이름을 모든 곳에 쓰면 대비가 깨진다 |
| 글꼴 파일·아이콘 패키지를 넣지 않는다. 한글 시스템 글꼴을 앞에 둔 스택, 아이콘은 JSX 안 SVG | CSP가 CDN을 막고, 파일을 넣으면 용량과 라이선스 관리가 생긴다 |
| 방송 상세는 1100px 이상에서 플레이어 + 360px 사이드바(후원 위, 댓글 아래), 좁으면 세로로 쌓는다. 상태 안내는 플레이어 자리에 같은 16:9 칸으로 | 시청 중 댓글·후원을 함께 본다. 상태가 바뀌어도 화면이 튀지 않는다 |
| 소유자는 ENDED 전까지 자기 방송 Player 대신 송출 패널(Publisher)을 본다 | media/9 Cautions의 "휴대폰 송출자가 올리면서 LL-HLS도 내려받는다"를 없앤다. 대신 같은 계정으로 다른 기기에서 자기 방송을 보는 길이 없어졌다 |
| 기존 문구를 유지한다(배지 `방송 중`·`준비` 등). 새 문구는 상단 바 `방송하기` 하나 | verify 여정과 README 확인 절차가 문구로 화면을 찾는다 |
| 화면 품질은 verify `layout` 여정으로 검사한다. 화면 × 뷰포트(1280x800, 390x844, 360x780, 짧은 1280x600) × 다크·라이트마다 가로 스크롤, 휴대폰 터치 대상 가로·세로 44px, 브라우저 기본 링크·버튼, WCAG 대비(계산값, 반투명 층 합성)를 본다 | 눈으로 보는 확인은 다시 돌릴 수 없다. 이 여정이 디자인 전 UI에서 실패하고(112/231) 후에 통과하는 것으로 완료 조건을 확인했다 |
| 댓글 시각은 오늘이면 `HH:mm`, 아니면 `M월 D일 HH:mm`, 전체 시각은 `title` (리뷰 F6) | 종료된 방송을 며칠 뒤 열면 날짜가 필요하다 |

## Rejected alternatives

- CSS 프레임워크(Tailwind 등): CSP와 이슈 제약상 추가하지 않는다. 토큰 + 손으로 쓴 CSS로 충분했다.
- 빈 상태 안의 행동 버튼(리뷰 F7): 상단 바와 홈에 같은 버튼이 있어 중복이다. design.md를 구현에 맞게 고쳤다.
- 짧은 창에서 사이드바 고정 높이를 푸는 CSS(리뷰 F3): 1280x600에서 댓글 입력이 첫 화면 안(하단 553px)이라 재현되지 않았다. 검사만 남겼다. 사이드바 안 스크롤은 댓글 목록 스크롤과 겹쳐 기각.
- 소유자에게 송출 패널과 Player를 함께 보이기(이전 동작): 위 이유로 기각. 필요해지면 송출 패널 아래에 붙인다.

## Cautions when touching this

- 색을 바꾸면 `run.sh layout`을 다시 돌린다. 대비는 계산된 스타일로 재므로 그라디언트는 가장 나쁜 정지점으로, `aria-hidden`과 disabled 텍스트는 빼고 잰다.
- 결제 확인 페이지는 `결제하기` 버튼이 1280x720 첫 화면 안에 있어야 donation 여정이 통과한다. 스크롤 직후 Playwright 클릭이 Toss 위젯에 닿지 않았다(원인 미확정, unverified: Toss iframe이 스크롤 중 움직인다는 추정).
- `pay-success`의 성공 안내는 실제 Toss 승인 뒤에만 보여 layout 여정에 없다.
- `[hidden]{display:none !important}`가 `.stage > video{display:block}`보다 이긴다. 이 규칙을 지우면 재생 실패 때 빈 video가 보인다.
- 1280x600보다 낮은 창(약 557px 미만, 측정값에서 추론, unverified)에서는 댓글 입력이 첫 화면 밖으로 밀릴 수 있다.
