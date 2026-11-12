---
issue: 58
area: auth
summary: users.role(streamer/viewer)를 두고 방송 생성만 유스케이스에서 streamer로 제한, 나머지 쓰기는 기존 소유권 검사에 맡긴다
status: confirmed
date: 2026-11-12
paths: [crates/domain/src/auth.rs, crates/application/src/broadcast.rs, crates/infrastructure/src/persistence/user.rs, crates/infrastructure/migrations/0010_user_roles.sql, web/src/auth/role.ts]
related: [1, 5]
---

# 사용자 역할(streamer / viewer)

## Background

역할이 없어서 가입한 모든 계정이 방송을 만들 수 있었다. 시청자 계정(`demo-viewer` 등)이 방송을 만들고 자기 방송을 수정·삭제하는 것은 "보기만 하는 사용자"라는 의도와 맞지 않았다. 다른 사람의 방송은 원래 소유권 검사(`owner_id` 조건부 UPDATE)로 막혀 있어서, 문제는 "누가 방송을 만들 수 있는가" 하나였다.

범위 밖: 관리자 등 다른 역할, 역할 변경 API, 권한 관리 화면.

## Decision and reasons

| Decision | Reason |
|---|---|
| 도메인 값 객체 `Role { Streamer, Viewer }`, `users.role`은 `TEXT NOT NULL DEFAULT 'viewer'` + CHECK | 두 값만 가능하다는 것을 타입과 DB 둘 다에서 보장한다. DB 문자열 변환은 infrastructure에서만 한다 |
| 가입 계정의 기본 역할은 `viewer`, 요청 DTO는 role을 받지 않는다 | 가입 때 역할을 고르게 하면 누구나 스스로 streamer가 되는 권한 상승 경로가 열린다 |
| 마이그레이션 0010에서 기존 계정을 모두 `streamer`로 올리고 `demo-viewer`만 `viewer`로 되돌린다 | 지금까지 방송을 만들 수 있던 계정은 계속 방송할 수 있어야 한다. 데모 시청자는 시연 목적상 viewer여야 한다 |
| 방송 생성만 `CreateBroadcast`에서 role을 검사한다. 거부는 `BroadcastError::NotStreamer`(403, `NOT_STREAMER`) | 인가는 유스케이스에서 한다는 규칙을 따른다. handler나 요청 DTO에 두지 않는다 |
| 수정·시작·종료·삭제에는 role 검사를 더하지 않는다 | 방송 소유자는 생성 시점에 streamer였던 계정뿐이고, 기존 `owner_id` 조건부 UPDATE가 이미 막는다. 같은 검사를 한 번 더 두면 중복이다 |
| 웹은 viewer에게 방송 만들기 진입점을 숨기고, 직접 URL로 들어오면 안내 화면을 보인다. 판단은 `canCreateBroadcast(user)` 하나 | UI는 안내용이고 실제 인가는 서버가 한다. 서버가 거부하면 `NOT_STREAMER` 한국어 문구가 나온다 |

## Rejected alternatives

- **모든 쓰기에 role 검사 추가**: 소유권 검사와 겹친다. 수정 경로마다 사용자 조회가 하나씩 늘어난다.
- **가입 때 역할 선택**: 위의 권한 상승 경로 때문에 기각.
- **기존 계정을 viewer로 마이그레이션**: 이미 방송하던 계정이 하루아침에 방송을 못 하게 된다.

감수한 한계: 가입 후 streamer가 되는 방법이 아직 없다. 역할 변경 API가 범위 밖이어서 지금은 운영자가 DB를 직접 고쳐야 하고, verify journey도 가입 뒤 compose Postgres에서 role을 올려 쓴다.

## Cautions when touching this

- 역할 변경 기능을 넣으면 `CreateBroadcast`가 role을 읽는 일과 방송을 넣는 일이 따로여서, 강등 직후의 요청이 생성에 성공할 수 있다. 그때 INSERT를 `users.role = 'streamer'` 조건부로 바꿔야 한다. 소유자만 쓰기가 가능하다는 위 전제도 다시 봐야 한다(강등된 streamer가 자기 방송을 계속 관리하게 둘지).
- 데모 계정의 role은 마이그레이션 0010에만 의존한다. 시드가 `ON CONFLICT DO NOTHING`이라 이미 있는 행의 role은 고치지 않는다. 데모 행을 수동으로 바꾸면 시드가 되돌려 주지 않는다.
- 새 에러 코드는 서버 영어 메시지 대신 웹 `MESSAGES`의 한국어 문구로 보인다. 코드를 추가하면 `web/src/api/client.ts`도 함께 고친다.
- viewer 안내 화면의 컴포넌트 테스트는 아직 없다. 판정 로직(`canCreateBroadcast`)만 테스트한다. 저장소에 컴포넌트 테스트 기반이 없어서다.
