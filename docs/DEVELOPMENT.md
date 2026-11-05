# 개발 규칙

이 저장소에서 개발할 때 따르는 절차와 이름 규칙. 목표와 우선순위는 [`intent.md`](../intent.md),
코드 설계 규칙(계층, 상태 전이, 인가, 비밀값)은 [`CLAUDE.md`](../CLAUDE.md)를 따른다.

## 1. 작업 흐름

```text
이슈 생성 → 브랜치 생성 → 최소 단위로 커밋 → PR → 리뷰 → squash merge → 브랜치 삭제 → 문서 갱신
```

- 모든 작업은 GitHub 이슈에서 시작한다. 이슈 없는 작업은 하지 않는다.
- 이슈 하나에 PR 여러 개가 붙을 수 있다. PR 하나는 한 가지 목적만 담는다.
- `main`에 직접 커밋하거나 push하지 않는다.
- Claude Code로 작업할 때는 `/issue-cycle <이슈 번호>` 한 번으로 구현 → 리뷰 → 리뷰 처리(수용/거부) → ADR → draft PR까지 진행한다. 절차는 [`.claude/skills/issue-cycle/SKILL.md`](../.claude/skills/issue-cycle/SKILL.md). 머지는 사람이 한다. 이 프로젝트 전용이며 Claude Code에 pstack과 jk 플러그인이 설치되어 있어야 한다.

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

- 브랜치는 항상 최신 `main`에서 만든다.
- 한 이슈에서 성격이 다른 PR을 내면 type으로 브랜치를 나눈다 (예: `feat/6`, `test/6`).
- 같은 이슈·같은 type의 PR을 또 내야 하면 앞 PR을 merge하고 브랜치를 지운 뒤 같은 이름으로 다시 만든다.
- merge 후 원격·로컬 브랜치를 지운다.

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

```bash
bash scripts/check-commit-msg.sh origin/main..HEAD
```

## 5. Pull Request

- 제목: 커밋 제목과 같은 형식 `<type>: <한국어 요약>`. squash merge 시 이 제목이 `main`의 커밋 제목이 된다.
- 본문: [PR 템플릿](../.github/PULL_REQUEST_TEMPLATE.md)을 채운다.
  - 관련 이슈: 이슈의 마지막 PR이면 `Closes #N`, 아니면 `Refs #N`.
  - 설계 결정이 있었으면 같은 PR에 ADR 파일을 넣고, `설계 기록(ADR)`에 그 파일 경로(`docs/adr/<area>/<issue>-<slug>.md`)를 적는다.
- CI가 모두 통과하고 리뷰를 마친 뒤 **squash merge**한다.
- 리뷰 지적을 반영한 커밋은 같은 브랜치에 추가하고, PR 본문 `수정 사항`에 적는다. force push는 리뷰 전까지만.

## 6. 검증

커밋 전에 로컬에서 돌린다. CI도 같은 검사를 한다.

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --workspace
cargo deny check
bash scripts/check-layer-deps.sh
```

`web/`을 바꿨다면:

```bash
cd web && npm run lint && npm run typecheck && npm run build
```

| CI job | 검사 |
|---|---|
| `rust` | fmt, clippy, test, 계층 의존성(`check-layer-deps.sh`) |
| `cargo-deny` | 라이선스, 취약점, 금지 crate |
| `gitleaks` | 비밀값 유출 |
| `commit-messages` | 커밋 메시지 형식 (PR만) |
| `web` | lint, typecheck, build |

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

작업 단위(PR)가 끝나면 Obsidian `개발/live-platform/`을 갱신한다. 저장소에는 들어가지 않는다.

| 폴더 | 언제 |
|---|---|
| `Rust-Learning/` | 새 Rust 개념·패턴을 썼을 때 (Java 관점 설명) |
| `Devlog/` | 매 작업일 |
| `Security/` | 위협 모델이 바뀌었을 때 |

저장소 안 문서(`docs/`, README)에는 비밀값, 내부 주소, 개인 정보를 넣지 않는다.
