# CLAUDE.md

이 저장소에서 작업하는 에이전트를 위한 규칙. 목표와 원칙은 `intent.md`가 우선한다.

## 프로젝트

치지직과 같은 실시간 방송 플랫폼. Rust 백엔드(axum) + React SPA + MediaMTX.
사용자는 Rust를 학습 중인 Java 개발자다. **코드를 쓰면 Java 관점 설명을 Obsidian에 남긴다.**

## 구조 (Cargo workspace, Hexagonal)

| crate | 역할 | 의존 가능 |
|---|---|---|
| `domain` | 엔티티, 값 객체, 상태 머신, 도메인 에러 | 없음 (std, 최소 유틸만) |
| `application` | 유스케이스, port trait | `domain` |
| `infrastructure` | sqlx, MediaMTX, S3, Toss, JWT adapter | `domain`, `application` |
| `api` | axum handler, DTO, DI 조립(main) | 전부 |

- `domain`·`application`에 인프라 이름(AWS, MediaMTX, Postgres, axum)이 나오면 안 된다.
- port는 작게 쪼갠다(ISP). 예: `IngestRouter`, `PublishAuthorizer`, `PlaybackResolver`, `StreamTerminator`.
- 주입은 `Arc<dyn Trait>`. async trait은 `#[async_trait]`.

## 반드시 지킬 규칙

- **상태 전이**: `UPDATE ... WHERE id=$1 AND owner_id=$2 AND status IN (...)` 형태의 조건부 UPDATE로만. 영향 행 0이면 도메인 에러(409/403/404).
- **수정 충돌**: `version` 컬럼 낙관적 락. **중복 방지**: UNIQUE + `ON CONFLICT DO NOTHING`. **단일 실행 작업**: PG advisory lock.
- **ID**: Snowflake(`BIGINT`). JSON에서는 **항상 문자열**로 직렬화.
- **인가**: handler가 아니라 유스케이스에서. 요청 DTO에서 `owner_id`/`status`/`version`을 바인딩하지 않는다.
- **SQL**: sqlx 바인딩만. 문자열 연결 금지.
- **비밀값**: `Secret<T>`로 감싸고 로그에 남기지 않는다. `.env`는 커밋 금지, `.env.example`에는 자리표시자만.
- **에러 응답**: `{code, message, traceId}`만. 스택·SQL 노출 금지.
- **CPU 무거운 작업**(argon2 등): `spawn_blocking` + 세마포어로 동시 실행 수 제한.

## 방송 상태

`READY → LIVE ⇄ INTERRUPTED(30s 유예) → ENDED`. 삭제는 READY/ENDED에서만(soft delete). 종료는 DB 커밋 후 MediaMTX kick.

## 컨테이너 (compose)

모든 서비스에 `cpus`/`mem_limit`/`pids_limit`/로그 로테이션/healthcheck/`restart: unless-stopped`를 둔다. non-root, read-only rootfs, 이미지 digest 고정. Postgres와 MediaMTX API는 외부 포트를 열지 않는다.

## 검증 명령

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --workspace
cargo deny check
```

## 문서

- **ADR(결정 기록)은 저장소 `docs/adr/<area>/<issue>-<slug>.md`**에 쓴다. 형식은 `docs/adr/README.md`. 결정을 구현한 PR에 함께 넣고, PR 본문 `설계 기록(ADR)`에 그 파일 경로를 적는다.
  - `jk:session-summary`: 스킬의 vault 경로(`<vault>/<project>/...`) 대신 **`docs/adr/<area>/<issue>-<slug>.md`**에 저장한다. 임시 파일 작성 → `scan-sensitive.sh` 통과 → 저장 순서는 그대로 지키고, 저장 후 `docs/adr/README.md` 목록에 한 줄 추가한다. 커밋은 작업 흐름대로 해당 PR에 포함한다.
  - Obsidian `ADR/<area>/<issue>-<slug>.md`에 같은 결정의 상세판(배경, 검토한 방안, 장단점, 선택·기각 이유, 구현 코드)을 함께 쓴다. 형식은 `.claude/skills/issue-cycle/obsidian-adr.md`. 저장소 ADR은 요약판으로 짧게 둔다.
  - `jk:session-context`: 인덱스는 저장소의 `docs/adr`에서 만든다.
    `OBSIDIAN_VAULT="$(git rev-parse --show-toplevel)/docs" bash "<SKILL_DIR>/scripts/build-index.sh" -p adr -m <area>,common -c main`
- Obsidian은 사용자 개인 노트: `C:\Users\akdir\OneDrive\문서\obsidian\문서관리\개발\live-platform\` (vault 루트는 `문서관리`)
  - `00-Overview/` 개요·Capacity Plan, `Rust-Learning/` Java↔Rust, `Devlog/` 날짜별 로그, `Security/` 위협 모델

## 작업 단위

- 모든 작업은 GitHub 이슈 단위. 이슈 하나에 PR 여러 개가 붙을 수 있다.
- 브랜치: `<type>/<issue>` (예: `feat/6`). 절차 전체는 `docs/DEVELOPMENT.md`. PR 본문에 `Refs #N`, 이슈의 마지막 PR에만 `Closes #N`.
- 저장소는 제출 전까지 private. 마감 전 gitleaks 이력 스캔 후 public 전환(#15).
- 작업 단위가 끝나면 Devlog와 관련 Rust-Learning 노트를 갱신한다.
- 오픈소스 사용 시 README `Credits`에 출처를 추가한다.

## 커밋

기본 브랜치에서 작업 중이면 브랜치를 먼저 만든다. 이슈 작업 중에는 최소 기능 단위마다 커밋하고, 각 커밋은 검증 명령을 통과한 상태여야 한다.

메시지 형식(`scripts/check-commit-msg.sh`가 PR CI에서 검사한다):

```text
<type>: <한국어 요약>

<왜 바꿨는지. 선택>

Refs #<이슈 번호>
```

- type: `feat` `fix` `refactor` `perf` `test` `docs` `build` `ci` `chore` 중 하나, 소문자, scope 없음.
- 요약은 한국어, 끝에 마침표 없음. type과 코드 식별자는 영어 그대로.
- 마지막 줄은 `Refs #N`. 이슈를 닫는 `Closes #N`은 PR 본문에만 쓴다.
- AI 작성자 표기(`Co-Authored-By` 등)는 넣지 않는다.
