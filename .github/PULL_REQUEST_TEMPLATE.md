<!--
브랜치 이름: <issue>-<slug> (예: 1-workspace-skeleton)
이슈에 붙는 PR이 여러 개면 앞선 PR은 `Refs #N`만, 이슈의 마지막 PR에만 `Closes #N`을 쓴다.
-->

Refs #

## 무엇을 바꿨나

## 왜

## 관련 ADR

<!-- Obsidian 경로 규칙: ADR/<area>/<issue>-<slug>.md (예: ADR/common/1-hexagonal-five-planes.md). 없으면 "없음". -->

## 검증

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --all-targets -- -D warnings`
- [ ] `cargo test --workspace`
- [ ] `cargo deny check`

## 체크리스트

- [ ] 이 PR이 이슈의 마지막 PR이면 `Refs #N`을 `Closes #N`으로 바꿨다
- [ ] `domain`·`application`에 인프라 이름(AWS, MediaMTX, Postgres, axum)이 없다
- [ ] 비밀값을 커밋하거나 로그에 남기지 않았다
- [ ] Devlog와 관련 Rust-Learning 노트를 갱신했다
