# Obsidian 상세 ADR 형식

저장소 `docs/adr/`의 ADR은 핵심을 빠르게 이해시키는 요약판이다. Obsidian의 ADR은 같은 결정을 학습·회고용으로 자세히 쓴 상세판이다.

- 경로: `<vault>/개발/live-platform/ADR/<area>/<issue>-<slug>.md`. 저장소 ADR과 area, 파일명이 같다.
- 독자: Rust를 배우는 Java(Spring) 개발자. 한국어로 쓰고 코드 식별자는 영어 그대로 둔다.

## 템플릿

````markdown
---
tags: [live-platform, adr]
issue: <N>
pr: <PR 번호>
area: <area>
status: <저장소 ADR의 status>
date: <저장소 ADR의 date>
repo_adr: docs/adr/<area>/<issue>-<slug>.md
---

# <제목>

> 요약판: 저장소 `docs/adr/<area>/<issue>-<slug>.md`. 이 노트는 상세판이다.

## 배경
무엇이 문제였는지, 어떤 제약이 있었는지.

## 검토한 방안
### 방안 A: <이름> (선택)
설명.
- 장점:
- 단점:
### 방안 B: <이름>
...

## 선택한 이유

## 다른 방안을 선택하지 않은 이유
방안마다 한 단락. 측정, 실제 실행 결과, 문서 같은 근거가 있으면 그대로 적는다.

## 구현
어떻게 구현했는지와 핵심 코드. 코드 블록 위에 `경로:줄`을 적고, 실제 코드에서 짧게 발췌한다.
Java/Spring으로 치면 무엇에 해당하는지 한두 줄 덧붙여도 좋다.

## Follow-up
저장소 ADR의 Follow-up을 날짜와 함께 옮기고, 각각 배경, 방안, 이유를 짧게 적는다.

## 관련
- 관련 ADR: [[<파일명>]]
- Rust 노트: [[<NN-...>]], Devlog: [[YYYY-MM-DD]]
````

## 규칙

- 사실의 출처는 저장소 ADR, PR 본문, 이슈 본문, 코드, git log, 그리고 결정이 나온 대화다. 출처에 없는 이유를 만들지 않는다. 추론으로 보충한 줄 끝에는 `(추론)`, 확인하지 못한 사실에는 `(미확인)`을 붙인다.
- 비밀값(키, 비밀번호, 토큰, `.env` 값)은 넣지 않는다.
- 저장 순서: 임시 파일에 초안 작성 → jk:session-summary의 `scan-sensitive.sh` 통과 → Obsidian 경로에 저장.
