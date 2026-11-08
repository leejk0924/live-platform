---
name: issue-cycle
description: 이슈 하나를 구현부터 draft PR까지 한 사이클로 진행한다. poteto-mode로 구현하고, jk:code-review로 리뷰하고, 리뷰 항목을 수용/거부로 처리한 뒤, jk:session-summary로 ADR을 남기고 커밋·push·draft PR까지 만든다. "/issue-cycle <이슈 번호>", "이슈 사이클 돌려" 같은 요청에 쓴다.
---

# Issue cycle

인자: 이슈 번호 (예: `/issue-cycle 6`). 없으면 묻는다.

한 사이클 = PR 하나. 사이클 전체를 **poteto-mode**로 진행한다. poteto-mode는 모델이 직접 호출할 수 없는 스킬이므로, 시작할 때 아래 명령으로 찾은 `SKILL.md`를 끝까지 읽고 그 규칙(원칙 인용, 검증, Autonomy)을 사이클 끝까지 따른다.

```bash
ls -d ~/.claude/plugins/cache/*/pstack/*/skills/poteto-mode/SKILL.md | sort -V | tail -1
```

규칙의 출처: 절차·이름은 `docs/DEVELOPMENT.md`, 코드 규칙은 `CLAUDE.md`. 이 파일과 충돌하면 그 두 문서가 이긴다.

## 0. 전제 확인

이 스킬은 이 프로젝트 전용이고 플러그인 두 개가 있어야 한다. 시작 전에 확인하고, 하나라도 없으면 아무 작업도 하지 않고 멈춘 뒤 무엇이 없는지 알린다.

- **pstack**: 위 `ls` 명령이 `poteto-mode/SKILL.md` 경로를 출력한다.
- **jk**: 사용 가능한 스킬 목록에 `jk:code-review`, `jk:session-summary`, `jk:session-context`가 있다.

## 1. 준비

1. `gh issue view <N>`으로 목표·범위·완료 조건을 읽는다. 완료 조건이 확인 가능한 문장이 아니면 사용자에게 묻고 멈춘다.
2. 이 PR이 이슈를 끝내는지 판단한다. 끝내면 PR 본문에 `Closes #N`, 아니면 `Refs #N`. 애매하면 `Refs`로 두고 보고에 적는다.
3. 최신 `main`에서 `<type>/<N>` 브랜치를 만든다. 같은 이름이 이미 있으면 멈추고 묻는다.
4. **jk:session-context**로 관련 ADR을 불러온다 (`CLAUDE.md`의 `docs/adr` 경로 규칙대로).

## 2. 구현 (poteto-mode)

- 이슈의 완료 조건을 finish condition으로 삼아 poteto-mode의 해당 playbook(Feature, Bug fix 등)대로 구현한다.
- 최소 기능 단위마다 커밋한다. 각 커밋은 `DEVELOPMENT.md` §6 검증 명령을 통과해야 하고 메시지는 §4 형식(`Refs #N`)을 따른다.
- 완료 조건마다 실제로 돌린 증거(명령과 출력)를 남겨 둔다. PR 본문에 쓴다.

## 3. 리뷰 (jk:code-review)

- **jk:code-review**를 현재 브랜치에 실행한다. PR이 아직 없으므로 의도(intent)는 1단계의 이슈 목표·완료 조건을 그대로 넘긴다.
- 결과의 지적사항 표(ID, 위치, 심각도, 지적, 근거, 제안, 처리)와 Repro 목록을 그대로 다음 단계에 넘긴다. 요약하거나 행을 빼지 않는다.
- Java·Rust 파일이 없어 리뷰가 멈추면 "리뷰 대상 아님"으로 기록하고 5단계로 간다.

## 4. 리뷰 처리 (poteto-mode)

poteto-mode에 리뷰 결과를 넣고 **각 항목을 수용/거부로 판단하고 이유를 남긴다**. 모든 행에 판단이 있어야 한다.

- **수용**: 고친다.
  - `proven`이 아닌 행은 Repro의 테스트를 먼저 쓰고 **실패하는 것을 확인한 뒤** 고치고, 통과를 확인한다.
  - 지적 하나(또는 같은 원인의 묶음)마다 커밋한다. 본문에 처리한 ID(`F2`)를 적는다.
- **거부**: 고치지 않는다. 이유는 의견이 아니라 근거로 쓴다: 반증하는 테스트 실행 결과, 이미 그 경우를 막는 코드 위치(`path:line`), 또는 이슈 범위 밖이라는 근거(이슈 문장 인용).
  - 범위 밖이지만 맞는 지적이면 거부하고 후속 이슈 후보로 적는다.
- 처리 열이 `ask`인 행(보안·데이터·동시성·마이그레이션 등)도 판단은 내리되 **확인 필요**로 표시한다. 사용자가 draft PR에서 확인한다.
- 처리가 끝나면 검증 명령을 다시 돌린다. 수용한 수정이 크면(새 파일, 계약 변경) jk:code-review를 한 번 더 돌리고 같은 방식으로 처리한다. 최대 2회.

결과 표 (PR 본문에 그대로 넣는다):

| ID | 판단 | 이유 | 커밋 |
|---|---|---|---|
| F1 | 수용 | `refresh_retry` 테스트 실패 확인 후 수정, 통과 | `abc1234` |
| F2 | 거부 | `order.rs:80`의 조건부 UPDATE가 이미 경쟁을 막음, `concurrent_order` 테스트 통과 | - |
| F3 | 수용 (확인 필요) | 세션 만료 처리 변경. 보안 항목이라 확인 요청 | `def5678` |

## 5. ADR (jk:session-summary)

- **jk:session-summary**를 실행한다. 저장 위치는 `CLAUDE.md` 규칙대로 `docs/adr/<area>/<N>-<slug>.md`이고, `docs/adr/README.md` 목록에 한 줄 추가한다.
- 리뷰 처리에서 설계가 바뀐 결정(거부 이유가 설계 판단인 것 포함)도 ADR에 넣는다.
- 기록할 결정이 없으면 ADR을 쓰지 않고 이유를 보고에 적는다.
- 커밋: `docs: <요약> ADR을 남긴다` + `Refs #N`.
- 같은 결정의 **상세판**을 Obsidian `ADR/<area>/<N>-<slug>.md`에 쓴다. 형식은 [obsidian-adr.md](obsidian-adr.md). 저장소 ADR은 요약판으로 둔다. 기존 ADR에 Follow-up을 붙였으면 상세판에도 같은 Follow-up을 붙인다.
- Obsidian `Devlog/`와 해당 `Rust-Learning/` 노트를 갱신한다. 각 섹션 제목에 브랜치와 PR 링크를 단다.
- Obsidian `00-Overview/Timeline.md`에 이 이슈를 한 줄 추가한다. 머지 열은 `(draft)`로 둔다.
- Obsidian 쪽은 모두 저장소 밖이라 커밋하지 않는다.

## 6. push와 draft PR

1. `bash scripts/check-commit-msg.sh origin/main..HEAD`가 통과해야 한다.
2. `git push -u origin <type>/<N>`.
3. `gh pr create --draft`. 제목은 커밋 형식 `<type>: <한국어 요약>`, 본문은 `.github/PULL_REQUEST_TEMPLATE.md`를 채운다.
   - `작업 내용`: 구현 내용과 완료 조건별 증거(명령과 결과).
   - `설계 기록(ADR)`: 5단계에서 만든 파일 경로. 없으면 줄을 지운다.
   - `수정 사항`: 4단계 결과 표. 리뷰를 건너뛰었으면 그 이유.
   - `관련 이슈`: 1단계에서 정한 `Closes #N` 또는 `Refs #N`.
   - `완료 조건`: 이슈의 완료 조건을 체크리스트로, 확인한 것만 체크.

## 7. 보고

사용자에게 짧게 알린다: draft PR 링크, 리뷰 처리 집계(수용 n / 거부 n / 확인 필요 n)와 확인 필요 행 목록, ADR 경로(저장소와 Obsidian), 후속 이슈 후보, 끝내 확인하지 못한 완료 조건. 머지는 하지 않는다.

## 8. 머지 후 (사용자가 머지를 요청했을 때만)

1. PR의 CI가 모두 통과했는지 확인한다. 코드와 무관한 실패(다운로드 실패 등)는 실패한 job만 다시 돌린다.
2. draft를 ready로 바꾸고 squash 머지한다. 본문은 이 PR의 커밋만 넣는다: `git log --no-merges --reverse --format='* %B' origin/main..origin/<type>/<N>`.
3. Obsidian을 PR의 최종 상태로 맞춘다.
   - `Devlog/`: 리뷰 수치를 PR 본문 `수정 사항` 표의 최종 행 수로 고치고, 섹션 제목에 머지 시각(KST)을 적는다.
   - `00-Overview/Timeline.md`: 이 이슈 줄의 `(draft)`를 머지 시각으로 바꾸고, 줄을 머지 순서에 맞는 자리로 옮긴다.
