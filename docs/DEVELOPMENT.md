# 개발 규칙

이 저장소에서 개발할 때 따르는 절차와 이름 규칙. 목표와 우선순위는 [`intent.md`](../intent.md),
코드 설계 규칙(계층, 상태 전이, 인가, 비밀값)은 [`CLAUDE.md`](../CLAUDE.md)를 따른다.

## 1. 작업 흐름

```text
이슈 생성 → 브랜치(worktree) 생성 → 최소 단위로 커밋 → PR → 문서 갱신 → 리뷰 → squash merge → worktree·브랜치 삭제 → 문서 마무리
```

- 모든 작업은 GitHub 이슈에서 시작한다. 이슈 없는 작업은 하지 않는다.
- 이슈 하나에 PR 여러 개가 붙을 수 있다. PR 하나는 한 가지 목적만 담는다.
- `main`에 직접 커밋하거나 push하지 않는다.
- Claude Code로 작업할 때는 `/issue-cycle <이슈 번호> [끝나는 조건]` 한 번으로 정한 단계까지 묻지 않고 진행한다. 끝나는 조건은 `구현까지`, `리뷰까지`, `PR까지`, `머지까지`이고, 생략하면 `PR까지`(구현 → 리뷰 → 리뷰 처리(수용/거부) → ADR → draft PR)다. 절차는 [`.claude/skills/issue-cycle/SKILL.md`](../.claude/skills/issue-cycle/SKILL.md). 머지는 `머지까지`를 주거나 사람이 요청할 때만 하고, `확인 필요` 행이 있으면 머지 전에 멈춰 사람이 다시 확인한 뒤에 한다. 이 프로젝트 전용이며 Claude Code에 pstack과 jk(0.4.0 이상) 플러그인이 설치되어 있어야 한다. 구현과 리뷰 수정은 `pstack:poteto-agent` 서브에이전트가 하고, 메인 대화가 diff를 검토하고 검증을 다시 돌린다.

## 2. 이슈

| 템플릿 | 언제 | 제목 |
|---|---|---|
| 작업 | 기본. 하나의 작업 단위 | 결과를 한 줄로 (예: `방송 CRUD + 인가 매트릭스 테스트`) |
| ✨ 기능 추가 | 새 기능 제안 | `[도메인] 기능 설명` |
| ♻️ 리팩토링 | 동작 변경 없는 구조 개선 | `[도메인] 리팩토링 대상` |

- 도메인: `auth` `broadcast` `media` `comment` `recording` `donation`. 특정 도메인이 아니면 `common`, 인프라·설정은 `infra`.
- 완료 조건은 확인 가능한 문장으로 쓴다. 검증 명령 통과는 항상 포함한다.

## 3. 브랜치

```text
<type>/<issue>
```

| 부분 | 규칙 | 예 |
|---|---|---|
| `type` | 커밋 type과 같은 집합: `feat` `fix` `refactor` `perf` `test` `docs` `build` `ci` `chore` | `feat` |
| `issue` | GitHub 이슈 번호 | `6` |

```text
feat/6
fix/10
docs/1
infra/2           ✗  infra는 type이 아니다 → build/2 또는 ci/2
6-broadcast-crud  ✗  type 없음
```

- 브랜치는 항상 최신 `main`에서 만든다. 메인 체크아웃 옆의 별도 worktree에서 작업한다. `<repo>`는 메인 체크아웃의 폴더 이름이다.
- 한 이슈에서 성격이 다른 PR을 내면 type으로 브랜치를 나눈다 (예: `feat/6`, `test/6`).
- 같은 이슈·같은 type의 PR을 또 내야 하면 앞 PR을 merge하고 브랜치를 지운 뒤 같은 이름으로 다시 만든다.
- merge 후 worktree와 원격·로컬 브랜치를 지운다(`git worktree remove`, `git branch -D`, `git push origin --delete`). 메인 체크아웃은 `git pull --ff-only`로 맞춘다.

worktree 만들기:

```bash
git fetch origin
git worktree add ../<repo>-<type>-<N> -b <type>/<N> origin/main
```

## 4. 커밋

```text
<type>: <한국어 요약>

<왜 바꿨는지. 선택>

Refs #<이슈 번호>
```

| type | 쓰임 |
|---|---|
| `feat` | 기능 추가 |
| `fix` | 버그 수정 |
| `refactor` | 동작 변경 없는 구조 개선 |
| `perf` | 성능 개선 |
| `test` | 테스트 추가·수정 |
| `docs` | 문서 |
| `build` | Cargo, npm, Docker, compose 등 빌드·배포 구성 |
| `ci` | GitHub Actions, 검사 스크립트 |
| `chore` | 그 밖의 잡무 (설정, 정리) |

- type은 소문자, scope 없음. 요약은 한국어, 끝에 마침표 없음. type과 코드 식별자는 영어 그대로.
- 본문은 *무엇*이 아니라 *왜*를 쓴다. 무엇은 diff가 보여 준다.
- 마지막 줄은 `Refs #N`. `Closes #N`은 커밋이 아니라 PR 본문에만 쓴다.
- AI 작성자 표기(`Co-Authored-By` 등)를 넣지 않는다.
- 최소 기능 단위마다 커밋하고, **각 커밋은 검증 명령을 통과한 상태**여야 한다.
- PR CI가 `scripts/check-commit-msg.sh`로 형식을 검사한다. push 전에 로컬에서도 돌릴 수 있다.
- clone마다 한 번 `git config core.hooksPath .githooks`를 실행한다. `.githooks/pre-commit`이 staged 변경을 gitleaks로 검사해 비밀값이 든 커밋을 막는다. gitleaks가 없으면 digest를 고정한 이미지를 docker로 돌린다.

```bash
bash scripts/check-commit-msg.sh origin/main..HEAD
```

## 5. Pull Request

- 제목: 커밋 제목과 같은 형식 `<type>: <한국어 요약>`. squash merge 시 이 제목이 `main`의 커밋 제목이 된다.
- 본문: [PR 템플릿](../.github/PULL_REQUEST_TEMPLATE.md)을 채운다.
  - 관련 이슈: 이슈의 마지막 PR이면 `Closes #N`, 아니면 `Refs #N`.
  - 설계 결정이 있었으면 같은 PR에 ADR 파일을 넣고, `설계 기록(ADR)`에 그 파일 경로(`docs/adr/<area>/<issue>-<slug>.md`)를 적는다.
- CI가 모두 통과하고 리뷰를 마친 뒤 **squash merge**한다. `수정 사항`에 `확인 필요` 행이 있으면 머지 전에 사람이 그 행을 확인한다.
- 리뷰 지적을 반영한 커밋은 같은 브랜치에 추가하고, PR 본문 `수정 사항`에 적는다. force push는 리뷰 전까지만.

## 6. 검증

커밋 전에 로컬에서 돌린다. CI도 같은 검사를 한다.

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
DATABASE_URL=postgres://postgres:test@localhost:55432/postgres cargo test --workspace -- --include-ignored
cargo deny check
bash scripts/check-layer-deps.sh
```

DB 테스트는 `#[ignore]`로 표시되어 있고 `--include-ignored`로 함께 돌린다. Postgres가 필요하다. CI와 같은 이미지로 테스트 전용 컨테이너를 띄운다. 비밀번호 `test`는 테스트 전용이다. 포트 55432는 compose의 Postgres와 겹치지 않게 고른 것이다.

```bash
docker run -d --name lp-test-pg -e POSTGRES_PASSWORD=test -p 55432:5432 postgres:17.11-trixie@sha256:748a18a54d3842d43bc156f0840d78029b8d349e0feca98c12b9c4ce3404a6b3
```

다음부터는 `docker start lp-test-pg`로 다시 켠다.

`web/`을 바꿨다면:

```bash
cd web && npm run lint && npm test && npm run typecheck && npm run build
```

| CI job | 검사 |
|---|---|
| `rust` | fmt, clippy, test(Postgres 서비스로 DB 테스트 포함), 계층 의존성(`check-layer-deps.sh`) |
| `cargo-deny` | 라이선스, 취약점, 금지 crate |
| `gitleaks` | 비밀값 유출 |
| `commit-messages` | 커밋 메시지 형식 (PR만) |
| `web` | npm audit(high 이상), lint, test, typecheck, build |

## 7. 코드 규칙 요약

상세는 [`CLAUDE.md`](../CLAUDE.md). 리뷰에서 가장 먼저 보는 항목만 적는다.

- 의존 방향은 `api → infrastructure → application → domain` 안쪽으로만. `domain`·`application`에 인프라 이름이 나오면 안 된다.
- 상태 전이는 조건부 `UPDATE ... WHERE ... AND status IN (...)`. 영향 행 0이면 도메인 에러.
- 인가는 유스케이스에서. 요청 DTO에 `owner_id`·`status`·`version`을 바인딩하지 않는다.
- SQL은 sqlx 바인딩만. 비밀값은 `Secret<T>`, 로그 금지. `.env` 커밋 금지.
- ID는 Snowflake `BIGINT`, JSON에서는 문자열.
- 새 의존성을 넣으면 `cargo deny check`를 통과시키고, 오픈소스 출처를 README `Credits`에 적는다.

## 8. 문서

### ADR (저장소)

대안을 비교해 결정했으면 `docs/adr/<area>/<issue>-<slug>.md`에 남기고, 그 결정을 구현한 PR에 함께 넣는다.
형식과 목록은 [`docs/adr/README.md`](adr/README.md).

- 확정된 ADR의 본문을 고쳐 쓰지 않는다. 결정이 바뀌면 파일 끝에 `## Follow-up (YYYY-MM-DD)`를 붙이거나, 새 ADR을 쓰고 기존 ADR의 `status`를 `superseded`로 바꾼다.
- 새 ADR을 쓰면 `docs/adr/README.md` 목록에 한 줄 추가한다.

### Obsidian (개인 노트)

PR을 만든 뒤 Obsidian `개발/live-platform/`을 갱신한다. PR 번호와 링크가 필요해서 PR보다 먼저 쓰지 않는다. 머지 후 PR의 최종 상태로 맞춘다. 저장소에는 들어가지 않는다.

| 폴더 | 언제 |
|---|---|
| `00-Overview/Timeline.md` | PR을 만들 때 한 줄 추가, 머지 후 머지 시각 기록. 이슈별 읽는 순서의 입구 |
| `ADR/` | 저장소에 ADR을 쓴 PR을 만들 때. 같은 결정의 상세판(배경, 검토한 방안, 장단점, 선택 이유, 기각 이유, 구현 코드). 형식은 `.claude/skills/issue-cycle/obsidian-adr.md` |
| `Rust-Learning/` | 새 Rust 개념·패턴을 썼을 때 (Java 관점 설명) |
| `Devlog/` | 매 작업일. 섹션마다 브랜치와 PR 링크, 머지 후 리뷰 수치를 PR 최종 표에 맞춤 |
| `Security/` | 위협 모델이 바뀌었을 때 |

저장소 ADR은 핵심을 빠르게 이해시키는 요약판이고, Obsidian `ADR/`은 학습·회고용 상세판이다. 결정의 원본은 저장소 ADR이다.

저장소 안 문서(`docs/`, README)에는 비밀값, 내부 주소, 개인 정보를 넣지 않는다.
