---
name: verify-broadcast-lifecycle
description: 방송 상태 머신(READY → LIVE ⇄ INTERRUPTED → ENDED, soft delete)을 소유자 start/stop/delete, MediaMTX auth·hook·kick, reaper 패스가 동시에 바꿀 때의 모든 실행 순서를 TLA+ 모델(TLC)로 검사한다. 방송 상태 전이 SQL, BroadcastEvent 전이표, TrackPublisher, StopBroadcast, Reaper::tick, 송출 티켓 redeem, mediamtx.yml의 hook·overridePublisher를 바꿨을 때, 또는 그 경합에 대한 주장을 하기 전에 쓴다.
---

# verify-broadcast-lifecycle

E2E(`verify-live-platform`)는 정해진 순서를 한 번씩 돈다. 이 스킬은 방송 한 건을 바꾸는 주체들(소유자, MediaMTX, hook 처리기, reaper)의 **모든 실행 순서**를 모델 안에서 검사한다. 모델은 `spec/BroadcastLifecycle.tla`, 도구는 TLA+ TLC 1.7.4다. TLC는 Docker의 고정된 JRE 이미지에서 돈다. Java를 설치할 필요가 없다.

명령은 모두 저장소 루트의 Git Bash에서 실행한다. `S=.claude/skills/verify-broadcast-lifecycle/scripts`.

## 검사하는 성질

| ID | 성질 (한 문장) | 깨지면 생기는 일 |
|---|---|---|
| P1 `EndedIsFinal` | ENDED가 된 방송은 어떤 순서로도 다른 상태가 되지 않는다 | 끝난 방송이 늦은 hook 하나로 다시 LIVE가 되어 목록에 뜬다 |
| P2 `LegalTransitions` | 상태는 `BroadcastEvent::transition`의 다섯 화살표로만 바뀐다 | READY에서 바로 ENDED가 되는 식의 상태가 생긴다 |
| P3 `DeleteOnlyReadyOrEnded`, `DeletedIsFrozen` | 삭제는 READY·ENDED에서만 커밋되고, 삭제된 방송은 다시 바뀌지 않는다 | 송출 중인 방송이 사라지거나, 삭제된 방송이 되살아난다 |
| P4 `NoAdmitWhenInactive` | MediaMTX는 활성(LIVE, INTERRUPTED)이 아닌 방송에 송출을 들이지 않는다 | 종료·삭제된 방송에 같은 사람이 다시 송출한다 |
| P5 `NoExpireOnAir` | reaper 패스는 자기가 송출 중으로 본 방송을 만료하지 않는다(보정이 만료보다 먼저) | 다시 연결한 송출자의 방송이 유예 끝에 끝나 버린다 |
| L1 `EndedStreamStops` | 방송이 ENDED이거나 삭제되면 그 송출은 결국 끊기고 다시 살아나지 않는다 | 끝난 방송의 영상이 인증 없는 시청 경로로 계속 공개된다 |
| L2 `InterruptedResolves` | INTERRUPTED는 영원히 계속되지 않는다(LIVE로 돌아가거나 ENDED가 된다) | 송출자가 떠난 방송이 "잠시 끊김"으로 영원히 남는다 |
| L3 `GhostLiveResolves` | 송출자가 없는 LIVE는 영원히 계속되지 않는다 | hook을 놓친 방송이 영원히 LIVE로 보인다 |

P는 안전 성질(나쁜 일이 한 번도 일어나지 않는다), L은 진행 성질(좋은 일이 결국 일어난다)이다. L은 아래 "가정"의 공정성 조건 아래에서만 성립한다.

## 언제 쓰나

변경이 아래 "검사 범위"의 파일을 건드리면 쓴다. 건드렸는지는 이렇게 본다.

```bash
bash $S/drift.sh          # 동기화 커밋 이후 바뀐 검사 대상 파일 (커밋 안 된 변경 포함). 바뀐 게 없으면 exit 0
bash $S/drift.sh --diff   # 그 diff 전체
```

바뀐 파일이 있으면 아래 "변경을 모델에 옮기기"를 먼저 하고 검사를 돌린다. 모델에 옮기지 않고 돌린 통과는 옛 코드에 대한 통과다.

## 실행

```bash
bash $S/check.sh     # 본 검사: 성질 9개, N=2 세션, 인프라 실패 1회. 약 4분. 끝 줄 MODEL OK
bash $S/mutants.sh   # 각 성질이 아직 실패할 수 있는지: 깨진 변형 7개가 모두 잡혀야 MUTANTS OK. 약 3분
bash $S/mutants.sh no-expire resume-from-ended   # 일부만
```

- 필요한 것: Docker(이 프로젝트의 compose 스택과 같은 것)와 첫 실행의 네트워크. `check.sh`가 `tla2tools.jar` 1.7.4를 `~/.cache/tlaplus`에 받아 sha256으로 확인하고, `eclipse-temurin:21-jre`를 digest로 고정해 실행한다.
- TLC 출력 전체는 `~/.cache/tlaplus/last-run.txt`다. `check.sh`는 그중 오류, 상태 수, 반례의 상태 줄만 화면에 보여 준다.
- 메모리가 모자라면(`Java ran out of memory`) `TLC_HEAP=5g bash $S/check.sh`. Docker Desktop의 메모리 한도보다 작아야 한다.
- 실행 하나만 돌린다. 두 검사가 `last-run.txt`를 같이 쓴다.

## 결과 읽기

- `MODEL OK (...)`: 아래 범위와 가정 안에서 성질 9개가 모든 순서에서 성립한다. 이 줄과 걸린 시간을 인용한다.
- `Error: Action property X is violated` / `Error: Temporal properties were violated`: `last-run.txt`의 `State 1:`부터가 반례다. 각 상태 위의 `<Action line ...>`이 그 단계의 action 이름이고, 아래 "검사 범위" 표로 코드 함수와 SQL을 찾는다. 진행 성질 반례는 끝에 `Back to state k`가 있다. k번 상태부터 끝까지가 영원히 반복되는 고리다.
- 반례는 **pstack `/statechart` 스킬**로 읽는다(사용자가 `/pstack:statechart`로 실행). 위반마다 가장 짧은 경로를 쉬운 말로 보여 주고, 어떤 변경이 그 위반을 고치는지도 보여 준다. 통과했을 때는 상태 사이의 이동을 그림으로 보여 준다. 명령과 옵션은 그 스킬의 도움말을 따른다.

### 진짜 버그인가, 모델 오류인가

반례의 각 단계를 코드에서 그대로 따라가 본다.

1. 각 단계의 action을 표에서 찾아 그 함수와 SQL을 연다.
2. 그 순서가 실제로 일어날 수 있는지 확인한다. 이때 볼 것은 세 가지다. 한 문장(UPDATE 하나, Control API 호출 하나)이 한 단계인가. reaper 트랜잭션이 행 잠금을 잡은 동안 다른 UPDATE가 기다리는가. 결과가 `now()`가 패스 시작 시각으로 고정되는 것과 맞는가.
3. 모든 단계가 코드로 설명되면 **진짜 결함**이다. 모델이나 성질을 고쳐 통과시키지 않는다. 반례를 코드의 말로 짧게 적어 사용자에게 보고하고, 제품 코드 수정은 별도 이슈로 연다(사용자가 시키기 전에는 고치지 않는다).
4. 코드에 없는 단계가 있으면 **모델 오류**다. 예: 실제로는 기다리는 UPDATE가 모델에서는 바로 실행된다, 한 문장을 두 단계로 나눴다. 모델을 코드에 맞게 고친다. 성질을 약하게 바꾸지 않는다. 고친 뒤에는 `mutants.sh`로 성질이 여전히 실패할 수 있는지 확인한다.
5. 진행 성질 반례의 고리가 "실패가 무한히 반복"하거나 "reaper가 멈춤"이면 공정성 가정 밖의 경로다. 그런 경로가 나오면 모델의 `Fairness`나 `Fail` 한도가 깨진 것이니 모델을 본다.

## 변경을 모델에 옮기기

1. `drift.sh --diff`로 바뀐 부분을 읽고, 아래 표에서 그 코드에 대응하는 action을 찾는다.
2. 동작이 바뀌었으면 그 action의 조건(WHERE)과 결과를 코드와 같게 고친다. 예: `BroadcastEvent::Resume`의 `from`을 바꾸면 `ArrivedResume`와 `RResume`의 `match`, `Edges`를 같이 고친다. 새 문장(UPDATE나 Control API 호출)이 생기면 새 action으로 넣고 `Next`와 공정성 묶음(`ReaperStep`, `StopStep`, `HookStep`)에 넣는다.
3. `check.sh`. 의도한 변경인데 성질이 깨지면 위 "진짜 버그인가"를 따른다. 성질 자체를 바꿔야 하는 변경(예: 삭제를 LIVE에서도 허용하기로 결정)이면 사용자 확인을 받은 뒤 성질과 그 mutant를 같이 고친다.
4. action을 바꾸거나 추가했으면 `mutants.sh`도 돌린다. 깨진 코드를 흉내 낸 mutant가 `MISSED`로 나오면 그 성질은 더 이상 아무것도 증명하지 못한다.
5. 모델이 새 커밋의 코드를 그대로 설명한다는 것을 **표의 모든 행에 대해 확인한 뒤에만** 아래 동기화 커밋을 그 커밋으로 올리고, 바뀐 줄 번호를 표에 반영한다. 검사가 통과했다는 것만으로는 올리지 않는다.

## 검사 범위

Sync commit: `875bb1faa76d40721400655ea762bf3395fd6d2e` (2026-11-12, 이 커밋의 코드와 모델을 표의 행마다 대조했다)

<!-- covered:start -->
- `crates/domain/src/broadcast.rs`
- `crates/application/src/live.rs`
- `crates/application/src/media.rs`
- `crates/application/src/reaper.rs`
- `crates/application/src/broadcast.rs`
- `crates/infrastructure/src/persistence/broadcast.rs`
- `crates/infrastructure/src/persistence/reaper.rs`
- `crates/infrastructure/src/persistence/publish_ticket.rs`
- `crates/infrastructure/src/media/mediamtx.rs`
- `crates/api/src/internal.rs`
- `crates/api/src/reaper.rs`
- `deploy/mediamtx/mediamtx.yml`
<!-- covered:end -->

| 모델 action | 코드 | SQL / 호출 |
|---|---|---|
| `Edges`, 각 `match`의 상태 집합 | `crates/domain/src/broadcast.rs:106` `BroadcastEvent::transition`, `:42` `DELETABLE`, `:46` `ACTIVE` | `status_names(...)`로 각 SQL의 `status = ANY(...)`에 바인딩 |
| `Start` | `crates/application/src/live.rs:43` `StartBroadcast::run` → `crates/infrastructure/src/persistence/broadcast.rs:215` `start` | `UPDATE ... SET status='LIVE', live_since=now() WHERE id AND owner_id AND status = ANY('{READY}') AND deleted_at IS NULL`. 이미 활성이면 전이 없이 토큰만 다시 준다(모델: 상태 변화 없음) |
| `Stop` | `live.rs:78` `StopBroadcast::run` → `broadcast.rs:231` `stop` | `UPDATE ... SET status='ENDED' WHERE ... status = ANY('{LIVE,INTERRUPTED}')` |
| `StopKickList`, `StopKickApply`, `StopKickFail` | `live.rs:99` `terminator.kick` → `crates/infrastructure/src/media/mediamtx.rs:151` `kick` | `GET /v3/webrtc/sessions/list` 후 그 경로의 세션마다 `POST /v3/webrtc/sessions/kick/{id}`. 실패는 보고만 하고 다시 시도하지 않는다 |
| `Delete` | `crates/application/src/broadcast.rs:119` `DeleteBroadcast::run` → `persistence/broadcast.rs:189` `soft_delete` | `UPDATE ... SET deleted_at=now() WHERE ... status = ANY('{READY,ENDED}') AND deleted_at IS NULL` |
| `Auth` | `crates/api/src/internal.rs:84` `auth` → `media.rs` `AdmitMediaAccess::publish` → `crates/infrastructure/src/persistence/publish_ticket.rs:28` `redeem` | `INSERT INTO publish_token_uses ... SELECT ... FROM broadcasts b WHERE b.status = ANY('{LIVE,INTERRUPTED}') AND b.deleted_at IS NULL ON CONFLICT (jti) DO NOTHING`. 잠금 없는 읽기라 reaper 잠금을 기다리지 않는다 |
| `Answer`, `Publish`, `Drop` | MediaMTX. `deploy/mediamtx/mediamtx.yml:39` `overridePublisher: false` | WHIP 응답 후 세션 목록에 들어간다(`Answer`). ICE 연결 후 경로가 available이 된다(`Publish`, 두 번째 송출자는 거절). 끊기면 `Drop` |
| `LoseAvailable`, `LoseUnavailable` | `mediamtx.yml:55`, `:58` `runOnAvailable`/`runOnUnavailable`의 `wget` | 보고가 사라짐(API 정지, 네트워크) |
| `ArrivedResume` | `internal.rs:163` `hook` → `crates/application/src/media.rs:181` `presence.resume` → `persistence/broadcast.rs:252` `presence` | `UPDATE ... SET status='LIVE', live_since=now() WHERE id AND status = ANY('{INTERRUPTED}') AND deleted_at IS NULL` |
| `ArrivedCheck` | `media.rs:187` `presence.is_active` → `persistence/broadcast.rs:282` | `SELECT EXISTS(... status = ANY('{LIVE,INTERRUPTED}') AND deleted_at IS NULL)` |
| `ArrivedKickList`, `ArrivedKickApply`, `ArrivedKickFail` | `media.rs:190` `terminator.kick` | `Stop`의 kick과 같음 |
| `LeftInterrupt` | `media.rs:182` `presence.interrupt` → `persistence/broadcast.rs:252` `presence` | `UPDATE ... SET status='INTERRUPTED', interrupted_at=now() WHERE ... status = ANY('{LIVE}')` |
| `RBegin` | `crates/application/src/reaper.rs:57` `tick` → `crates/infrastructure/src/persistence/reaper.rs:54` `begin` | `BEGIN; SELECT pg_try_advisory_xact_lock(...)`. 이 트랜잭션의 `now()`가 패스 시작 시각 |
| `RProbeOk`, `RProbeFail` | `reaper.rs:62` `probe.live_paths` → `mediamtx.rs:172` | `GET /v3/paths/list`, `available == true`. 실패하면 보정과 `not_active`를 건너뛰고 만료는 한다 |
| `RResume` | `reaper.rs:68` → `persistence/reaper.rs:75` `reconcile` 첫 UPDATE | `WHERE status = ANY('{INTERRUPTED}') AND id = ANY(on_air) AND interrupted_at < now()` |
| `RInterrupt` | 같은 함수 두 번째 UPDATE | `WHERE status = ANY('{LIVE}') AND NOT (id = ANY(on_air)) AND live_since < now() - 30s` |
| `RNotActive` | `reaper.rs:69` → `persistence/reaper.rs:120` `not_active` | `SELECT on_air.id ... WHERE NOT EXISTS (... status = ANY(ACTIVE) AND deleted_at IS NULL)` |
| `RExpire` | `reaper.rs:73` → `persistence/reaper.rs:137` `expire` | `WHERE status = ANY('{INTERRUPTED}') AND interrupted_at < now() - 30s` → ENDED |
| `RDemoReset`, `RDemoSkip` | `persistence/reaper.rs:183` `reset_demo_broadcasts` | 데모 계정의 오늘 이전 방송 중 `status = ANY('{READY,ENDED}')`를 soft delete. 데모인지는 모델에서 열어 둔다 |
| `RCommit`, `RAbort` | `reaper.rs:90` `pass.finish` → `persistence/reaper.rs:204` | `COMMIT`. 패스 실패나 종료 시 대기(`crates/api/src/reaper.rs` `SHUTDOWN_WAIT`) 중 중단되면 `ROLLBACK` |
| `RKickList`, `RKickApply`, `RKickFail` | `reaper.rs:93` 만료·not_active 방송마다 `terminator.kick` | `Stop`의 kick과 같음. 커밋 뒤에 한다 |
| `Tick` | `domain/src/broadcast.rs:25` `INTERRUPT_GRACE` 30초 | `live_since`/`interrupted_at`이 유예보다 오래됨 |

PostgreSQL READ COMMITTED는 이렇게 옮겼다. reaper 패스는 트랜잭션 하나다. 커밋 전까지 다른 쪽은 그 쓰기를 보지 못한다(`rStatus`, `rDeleted`). 패스가 행을 고친 뒤(`rHolds`) 다른 UPDATE의 WHERE가 커밋된 행에 맞으면, 그 UPDATE는 커밋까지 기다렸다가 새 행으로 다시 판단한다. 맞지 않으면 기다리지 않고 0행으로 끝난다(`NotBlocked`).

## 범위와 가정

- **규모**: 방송 1건, 송출 시도(WHIP 세션) 2개(`N = 2`), 인프라 실패 1회(`MaxFailures = 1`). 인프라 실패는 probe 실패, kick 실패, 패스 롤백을 합친 것이다. 이 범위의 모든 순서를 검사한다. 세션 3개 이상이나 실패 2회 이상은 검사하지 않았다.
- **공정성(진행 성질 L1~L3의 전제)**: reaper는 계속 돈다. 시작한 kick은 끝난다. 잃어버리지 않은 hook은 행 잠금이 풀리면 처리된다. 시간은 흐른다. 실패는 유한 번이다. 소유자와 송출자의 행동, hook 전달 여부에는 아무것도 가정하지 않는다.
- **시간**: 초 단위 시계는 없다. `Tick`이 유예 경과를 표시하고, 패스는 시작할 때의 경과를 스냅숏으로 쓴다. 패스 시작 뒤에 쓰인 타임스탬프는 패스의 `now()`보다 새롭다고 본다. 몇 ms 차이로 시작한 트랜잭션 사이의 `now()` 역전은 다루지 않는다.
- **티켓**: 송출자는 언제든 유효한 토큰을 가진 것으로 본다(`Start`가 다시 발급한다). `jti` 1회 사용, 서명, 만료, 경로 검사는 모델 밖이다. 상태 검사(`b.status = ANY(ACTIVE)`)만 모델에 있다.
- **kick 부분 실패**: 세션 여러 개 중 일부만 끊기는 경우는 전부 실패로 근사했다.
- **모델 밖**: 소유자당 활성 방송 1개(부분 UNIQUE 인덱스), 제목 수정과 `version`, 녹화 세그먼트, 댓글, 후원, 시청(WHEP/LL-HLS), 휴대폰 UI의 재송출 판단(`web/src/broadcasts/publish/phase.ts`), MediaMTX 자체의 버그, 다른 방송의 경로로 들어온 hook.
- 통과는 이 모델과 이 범위에 대한 것이다. 시스템 전체가 맞다는 뜻이 아니다.

## 파일

| 파일 | 내용 |
|---|---|
| `spec/BroadcastLifecycle.tla` | 모델과 성질 |
| `spec/BroadcastLifecycle.cfg` | 본 검사 설정(N, MaxFailures, 성질 목록) |
| `spec/Mutants.tla`, `spec/mutants/*.cfg` | 성질마다 그 성질을 깨는 코드 변형 하나와, 그것만 검사하는 설정 |
| `scripts/check.sh` | TLC 실행. `check.sh <spec dir> <cfg> [module]`로 다른 설정·모듈도 돌린다 |
| `scripts/mutants.sh` | 모든 mutant가 잡히는지 확인 |
| `scripts/drift.sh` | 동기화 커밋 이후 검사 대상 코드 변경 |
