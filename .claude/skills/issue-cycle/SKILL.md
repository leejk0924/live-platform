---
name: issue-cycle
description: 이슈 하나를 구현부터 draft PR까지(끝나는 조건을 주면 구현, 리뷰, 머지까지) 묻지 않고 한 사이클로 진행한다. poteto-mode로 구현하고, jk:code-review 리뷰를 수용/거부로 처리하고, ADR과 draft PR을 만든다. "/issue-cycle <이슈 번호> [끝나는 조건]", "/issue-cycle 8 구현까지", "이슈 8 머지까지 돌려", "이슈 사이클 돌려" 같은 요청에 쓴다.
---

# Issue cycle

인자: `<이슈 번호> [끝나는 조건]` (예: `/issue-cycle 6`, `/issue-cycle 6 머지까지`). 이슈 번호가 없으면 묻는다.

한 사이클 = PR 하나. 사이클 전체를 **poteto-mode**로 진행한다. poteto-mode는 모델이 직접 호출할 수 없는 스킬이므로, 시작할 때 아래 명령으로 찾은 `SKILL.md`를 끝까지 읽고 그 규칙(원칙 인용, 검증, Autonomy)을 사이클 끝까지 따른다.

```bash
ls -d ~/.claude/plugins/cache/*/pstack/*/skills/poteto-mode/SKILL.md | sort -V | tail -1
```

규칙의 출처: 절차·이름은 `docs/DEVELOPMENT.md`, 코드 규칙은 `CLAUDE.md`. 이 파일과 충돌하면 그 두 문서가 이긴다.

## 끝나는 조건

| 조건 | 마지막 단계 | 끝났을 때 상태 |
|---|---|---|
| `구현까지` | 2 | 완료 조건마다 증거가 있고 `DEVELOPMENT.md` §6 검증이 통과한다. 커밋은 worktree에만 있다 |
| `리뷰까지` | 4 | 모든 리뷰 행에 판단이 있고, 수정이 커밋되고, 검증을 다시 돌려 통과한다. push 전이다 |
| `PR까지` (생략 시) | 8 | draft PR, Obsidian 갱신, 보고 |
| `머지까지` | 9 | squash 머지, worktree·브랜치 정리, Obsidian 최종 상태 |

- "기능이 정상 동작할 때까지"처럼 같은 뜻의 말은 `구현까지`로 본다.
- 알아볼 수 없는 조건이면 아무 작업도 하기 전에 넷 중 무엇인지 한 번 묻는다.

## 멈추는 곳

시작하면 끝나는 조건까지 단계 사이에서 묻지 않고 진행한다. 멈추는 곳은 아래뿐이다.

- 0단계: 플러그인이 없거나 jk 버전이 맞지 않는다.
- 1단계 1: 완료 조건이 확인 가능한 문장이 아니다.
- 1단계 3: 같은 이름의 브랜치나 worktree 폴더가 이미 있다.
- 9단계 1: `수정 사항` 표에 `확인 필요` 행이 있다. `머지까지`여도 머지 전에 멈추고 행 목록을 보여 준 뒤 확인을 기다린다. 해당 행이 없으면 묻지 않고 머지한다.

끝나는 조건 전에 멈추면 8단계 보고에 멈춘 단계, 이유, 이어 갈 명령이나 답을 적는다. 예: "`확인 필요` 행 F3을 확인했다"고 답하면 9단계 2부터 이어 간다. 이슈의 완료 조건을 고쳤으면 `/issue-cycle <N> <조건>`을 다시 실행한다.

## 0. 전제 확인

이 스킬은 이 프로젝트 전용이고 플러그인 두 개가 있어야 한다. 시작 전에 확인하고, 하나라도 없으면 아무 작업도 하지 않고 멈춘 뒤 무엇이 없는지 알린다.

- **pstack**: 위 `ls` 명령이 `poteto-mode/SKILL.md` 경로를 출력한다.
- **jk**: 0.4.0 이상이고, 사용 가능한 스킬 목록에 `jk:code-review`, `jk:session-summary`, `jk:session-context`가 있다.
- **jk 버전 일치**: 세션이 불러온 jk 스킬 경로(각 스킬의 base directory, 예: `.../plugins/cache/open-claude/jk/<version>/skills/...`)의 `<version>`이 `~/.claude/plugins/installed_plugins.json`의 `jk@open-claude` `version`과 같다. 세션은 시작할 때의 플러그인 버전을 계속 쓴다. 더 낮은 버전을 불러왔으면 멈추고 Claude Code를 다시 시작하라고 알린다.

```bash
grep -A4 '"jk@open-claude"' ~/.claude/plugins/installed_plugins.json | grep '"version"'
```

## 1. 준비

1. `gh issue view <N>`으로 목표·범위·완료 조건을 읽는다. 완료 조건이 확인 가능한 문장이 아니면 사용자에게 묻고 멈춘다.
2. 이 PR이 이슈를 끝내는지 판단한다. 끝내면 PR 본문에 `Closes #N`, 아니면 `Refs #N`. 애매하면 `Refs`로 두고 보고에 적는다.
3. 최신 `main`에서 `<type>/<N>` 브랜치를 별도 worktree로 만든다. `<repo>`는 메인 체크아웃의 폴더 이름이다. 같은 이름의 브랜치나 폴더가 이미 있으면 멈추고 묻는다.

   ```bash
   git fetch origin
   git worktree add ../<repo>-<type>-<N> -b <type>/<N> origin/main
   ```

   이후 단계는 모두 이 worktree에서 진행한다.
4. **jk:session-context**로 관련 ADR을 불러온다 (`CLAUDE.md`의 `docs/adr` 경로 규칙대로).

## 2. 구현 (poteto-mode)

- 이슈의 완료 조건을 finish condition으로 삼아 poteto-mode의 해당 playbook(Feature, Bug fix 등)대로 구현한다.
- 메인 대화는 설계를 하고, 구현은 `pstack:poteto-agent` 서브에이전트에 맡긴다. 지시문에는 바꿀 파일, 데이터 형태, 이미 내린 결정, 검증 명령(`DEVELOPMENT.md` §6)을 적는다.
- 서브에이전트가 돌아오면 메인 대화가 diff를 직접 읽고 검증 명령을 직접 다시 돌린다. 서브에이전트의 보고를 확인 없이 그대로 옮기지 않는다.
- 최소 기능 단위마다 커밋한다. 각 커밋은 `DEVELOPMENT.md` §6 검증 명령을 통과해야 하고 메시지는 §4 형식(`Refs #N`)을 따른다.
- 완료 조건마다 실제로 돌린 증거(명령과 출력)를 남겨 둔다. PR 본문에 쓴다.

## 3. 리뷰 (jk:code-review)

- **jk:code-review**를 현재 브랜치에 실행한다. PR이 아직 없으므로 의도(intent)는 1단계의 이슈 목표·완료 조건을 그대로 넘긴다.
- 결과의 지적사항 표(ID, 위치, 심각도, 지적, 근거, 제안, 처리)와 Repro 목록을 그대로 다음 단계에 넘긴다. 요약하거나 행을 빼지 않는다.
- 리뷰 대상 언어는 Java, Rust, TypeScript다 (jk 0.4.0). diff에 이 언어가 하나도 없어 리뷰가 멈출 때만 "리뷰 대상 아님"으로 기록하고 5단계로 간다.
- 재리뷰의 ID는 직전 리뷰의 마지막 ID + 1부터 빈 번호 없이 잇는다.

## 4. 리뷰 처리 (poteto-mode)

poteto-mode에 리뷰 결과를 넣고 **각 항목을 수용/거부로 판단하고 이유를 남긴다**. 모든 행에 판단이 있어야 한다.

- **수용**: 고친다.
  - 수정은 새 `pstack:poteto-agent`에 맡긴다. 지시문에는 수용한 행, 이미 내린 결정, 실패하는 테스트를 먼저 쓰는 규칙을 적는다. 돌아온 diff와 검증은 2단계처럼 메인 대화가 직접 확인한다.
  - 예외: 두 번째(마지막) 리뷰의 수정이 각각 몇 줄이면 메인 대화가 직접 고쳐도 된다. 그렇게 했으면 보고에 적는다.
  - `proven`이 아닌 행은 Repro의 테스트를 먼저 쓰고 **실패하는 것을 확인한 뒤** 고치고, 통과를 확인한다.
  - 지적 하나(또는 같은 원인의 묶음)마다 커밋한다. 본문에 처리한 ID(`F2`)를 적는다.
- **거부**: 고치지 않는다. 이유는 의견이 아니라 근거로 쓴다: 반증하는 테스트 실행 결과, 이미 그 경우를 막는 코드 위치(`path:line`), 또는 이슈 범위 밖이라는 근거(이슈 문장 인용).
  - 범위 밖이지만 맞는 지적이면 거부하고 후속 이슈 후보로 적는다.
- 처리 열이 `ask`인 행(보안·데이터·동시성·마이그레이션 등)도 판단은 내리되 **확인 필요**로 표시한다. 사용자가 draft PR에서 확인한다.
- 처리가 끝나면 `DEVELOPMENT.md` §6 검증 명령(DB 테스트 포함)을 다시 돌린다. 수용한 수정이 크면(새 파일, 계약 변경) jk:code-review를 한 번 더 돌리고 같은 방식으로 처리한다. 최대 2회.

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
- Obsidian 쪽은 PR 번호와 링크가 필요하므로 7단계에서 한다.

## 6. push와 draft PR

1. `bash scripts/check-commit-msg.sh origin/main..HEAD`가 통과해야 한다.
2. `git push -u origin <type>/<N>`.
3. `gh pr create --draft`. 제목은 커밋 형식 `<type>: <한국어 요약>`, 본문은 `.github/PULL_REQUEST_TEMPLATE.md`를 채운다. jk `pr-create`는 쓰지 않는다.
   - `작업 내용`: 구현 내용과 완료 조건별 증거(명령과 결과).
   - `설계 기록(ADR)`: 5단계에서 만든 파일 경로. 없으면 줄을 지운다.
   - `수정 사항`: 4단계 결과 표. 리뷰를 건너뛰었으면 그 이유.
   - `관련 이슈`: 1단계에서 정한 `Closes #N` 또는 `Refs #N`.
   - `완료 조건`: 이슈의 완료 조건을 체크리스트로, 확인한 것만 체크.

## 7. Obsidian

draft PR이 생긴 뒤에 한다. PR 번호와 링크를 여기서 처음 알 수 있다.

1. 5단계 ADR의 **상세판**을 Obsidian `ADR/<area>/<N>-<slug>.md`에 쓴다. 형식은 [obsidian-adr.md](obsidian-adr.md). 저장소 ADR은 요약판으로 둔다. 기존 ADR에 Follow-up을 붙였으면 상세판에도 같은 Follow-up을 붙인다. 5단계에서 ADR을 쓰지 않았으면 건너뛴다.
2. Obsidian `Devlog/`와 해당 `Rust-Learning/` 노트를 갱신한다. 각 섹션 제목에 브랜치와 PR 링크를 단다.
3. Obsidian `00-Overview/Timeline.md`에 이 이슈를 한 줄 추가한다. 머지 열은 `(draft)`로 둔다.

Obsidian 쪽은 모두 저장소 밖이라 커밋하지 않는다.

## 8. 보고

끝나는 조건의 마지막 단계를 마치거나 멈춘 곳에서 사용자에게 짧게 알린다. `머지까지`면 9단계를 마친 뒤에 알린다. 아직 없는 항목은 뺀다 (예: `구현까지`에는 PR 링크가 없다).

- 커밋 목록과 완료 조건별 검증 증거
- draft PR 링크
- 리뷰 처리 집계(수용 n / 거부 n / 확인 필요 n)와 확인 필요 행 목록
- ADR 경로(저장소와 Obsidian)
- 후속 이슈 후보, 끝내 확인하지 못한 완료 조건
- `PR까지`보다 앞에서 끝났으면 `PR까지` 남은 단계
- 멈췄으면 멈춘 단계, 이유, 이어 갈 명령이나 답

## 9. 머지와 정리 (`머지까지`이거나 사용자가 머지를 요청했을 때만)

1. PR 본문 `수정 사항` 표에서 `확인 필요`로 표시한 행을 사용자에게 보여 주고, 사용자가 다시 확인한 뒤에만 머지한다. 이 목록을 보기 전에 받은 머지 요청(`머지까지` 인자 포함)은 확인이 아니다. 해당 행이 없으면 바로 다음으로 간다.
2. PR의 CI가 모두 통과했는지 확인한다. 코드와 무관한 실패(다운로드 실패 등)는 실패한 job만 다시 돌린다.
3. draft를 ready로 바꾸고 squash 머지한다. 본문은 이 PR의 커밋만 넣는다: `git log --no-merges --reverse --format='* %B' origin/main..origin/<type>/<N>`.
4. worktree와 브랜치를 지우고 메인 체크아웃을 최신으로 맞춘다. 메인 체크아웃에서 실행한다. squash 머지는 브랜치를 merged로 보지 않으므로 `-D`를 쓴다.

   ```bash
   git worktree remove ../<repo>-<type>-<N>
   git branch -D <type>/<N>
   git push origin --delete <type>/<N>
   git checkout main && git pull --ff-only
   ```

5. Obsidian을 PR의 최종 상태로 맞춘다.
   - `Devlog/`: 리뷰 수치를 PR 본문 `수정 사항` 표의 최종 행 수로 고치고, 섹션 제목에 머지 시각(KST)을 적는다.
   - `00-Overview/Timeline.md`: 이 이슈 줄의 `(draft)`를 머지 시각으로 바꾸고, 줄을 머지 순서에 맞는 자리로 옮긴다.
