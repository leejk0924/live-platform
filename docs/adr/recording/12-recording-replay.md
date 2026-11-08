---
issue: 12
area: recording
summary: MediaMTX가 fMP4 세그먼트를 스풀에 쓰고 hook으로 알리면 API가 recordings 행을 만들고 lease claim으로 저장소(로컬·S3)에 옮긴다. 다시보기는 5분짜리 서명 URL(로컬 HMAC·CloudFront ECDSA)로 일반 video에서 재생한다
status: confirmed
date: 2026-11-09
paths: [crates/domain/src/recording.rs, crates/application/src/recording.rs, crates/application/src/ports/recording.rs, crates/infrastructure/migrations/0008_recordings.sql, crates/infrastructure/src/persistence/recording.rs, crates/infrastructure/src/recording, crates/api/src/recording.rs, crates/api/src/recording_store.rs, crates/api/src/internal.rs, deploy/mediamtx/mediamtx.yml, compose.yml, web/src/recordings]
related: [1, 8, 10, 14]
---

# 녹화와 다시보기

## Background

끝난 방송을 다시 볼 수 있어야 한다. media/1은 "MediaMTX 녹화 → `RecordingStorage`(S3) → CloudFront 서명 URL"로 방향만 정했다. 이번 실행은 실제 AWS를 쓰지 않는다는 제약이 있어, S3와 CloudFront는 코드와 가짜 엔드포인트 테스트까지만 하고 로컬은 파일시스템 adapter로 끝까지 검증했다.

## Decision and reasons

| 결정 | 이유 |
|---|---|
| MediaMTX `recordFormat: fmp4` 파일을 그대로 다시보기 소스로 쓰고 브라우저는 일반 `<video src>`로 재생한다. hls.js나 HLS 패키징을 쓰지 않는다 | 헤드리스 Chrome 156에서 MediaMTX 1.21.3 fMP4 녹화(H264+Opus)가 길이를 알고 탐색도 됐다(11.9초 파일, 8초로 이동 확인). 패키징 단계와 세그먼트별 서명이 필요 없다 |
| 녹화 1건 = MediaMTX 세그먼트 1개. 방송 하나에 여러 건이 생긴다(재송출, 30분 상한) | 송출이 끊겼다 이어지면 MediaMTX가 새 파일을 만든다. 합치려면 재다중화가 필요하다. 30분 상한은 파일 하나를 약 0.6GB(2.5Mbps)로 묶는다 |
| `runOnRecordSegmentComplete` hook(`segment_complete`)은 등록만 하고 바로 204를 준다. 복사는 백그라운드 storer가 한다. 12시간을 넘는 길이 보고는 거부한다 | 큰 파일 복사가 요청 10초 상한(common/27)에 걸린다. hook은 힌트이고, `source UNIQUE` + `ON CONFLICT DO NOTHING`으로 중복 보고가 한 행이 된다 |
| hook이 보낸 경로는 스풀 루트 아래 `<stream path>/<이름>.mp4`만 받는다(`FsSegmentSpool`) | 공유 비밀값이 있어도 경로 순회로 임의 파일을 저장소에 올리지 못하게 한다 |
| recordings 상태는 `PENDING → READY | FAILED`. storer는 `retry_at`을 lease로 쓰는 `FOR UPDATE SKIP LOCKED` claim으로 한 건씩 가져가고, 결과 기록은 `status = ANY(STORABLE)` 조건부 UPDATE다. 실패 기록(`release_failed`)에만 `attempts = $n`을 더한다 | 인스턴스가 여럿이어도 한 건을 한 곳만 맡고, crash하면 lease가 지난 뒤 다시 가져간다. 늦은 storer의 실패가 새 claim을 FAILED로 만들지 못하게 하되(리뷰 F5), 늦은 storer의 성공은 받는다. 어느 시도든 같은 세그먼트를 같은 키로 쓰기 때문이다(리뷰 F19) |
| 실패는 30초부터 두 배씩 늘려 다시 시도하고 5번째에 FAILED | 저장소 일시 장애는 회복하고, 영구 실패는 끝없이 돌지 않는다 |
| 저장 키는 ID로만 만든다(`recordings/<broadcast>/<recording>.mp4`) | 외부 문자열이 키에 들어가지 않는다 |
| 다시보기 URL 수명은 5분 고정. 플레이어는 1분 넘게 쓴 URL에서 미디어 오류가 나면 만료로 보고 그 위치에서 새 URL을 받는다. 새 URL이 1분 안에 다시 실패하면 멈추고 "다시 시도"를 보인다 | 처음에는 "5분 + 녹화 길이"였는데, 길이에 상한이 없고 CloudFront URL은 방송 삭제 뒤에도 그만큼 살아 있었다(리뷰 F3, F4). 고정 5분이면 삭제 뒤 CDN 노출이 5분 안으로 묶인다. 재요청 판단을 재생 진행으로 하면 되감기나 일시정지 뒤의 정상 만료가 실패가 되어(리뷰 F24, F25) URL을 쓴 시간으로 바꿨다. 바로 다시 실패하면 멈추는 것은 무한 재요청을 막는다(리뷰 F1) |
| 로컬 저장소는 `/api/recordings/{id}/file?expires&signature`(HMAC-SHA256)로 API가 직접 Range 응답을 한다. 매 요청 DB로 삭제·READY를 다시 확인한다 | 같은 출처라 CSP `media-src 'self'`로 충분하다. 매 요청 확인으로 삭제 즉시 이미 발급한 URL도 막힌다(e2e: 삭제 뒤 replay·file·상세 모두 404) |
| 로컬 서명 키는 프로세스 시작 때 무작위로 만든다 | `.env`에 새 비밀값이 필요 없다. 재시작하면 URL이 무효가 되지만 플레이어가 새 URL을 받는다(e2e에서 위조 URL로 같은 경로 확인) |
| CloudFront 서명은 RSA 대신 ECDSA P-256 + SHA-256(`Hash-Algorithm=SHA256`)이다 | `rsa` crate는 RUSTSEC-2023-0071(Marvin, 수정 버전 없음)이고 `/replay`는 누구나 서명 시간을 잴 수 있다. CloudFront는 2025-09부터 ECDSA 키를 받는다(AWS 문서). openssl `dgst -sha256 -verify`로 서명 호환을 확인했다 |
| S3 업로드는 rusty-s3 presigned PUT + reqwest 스트리밍(Content-Length). 자격증명은 정적 키만 | 세그먼트가 수백 MB라 메모리에 올리지 않는다. aws-sdk는 무겁다. IAM Role(IMDSv2)은 실제 EC2 없이 확인할 수 없어 후속으로 둔다 |
| 녹화 조회는 누구나(로그인 없이) 한다. 목록·상세·replay는 `broadcasts.deleted_at IS NULL` 조인으로 삭제된 방송의 녹화를 404로 만든다 | 라이브 시청(media/8)과 같은 공개 범위. 삭제 규칙을 SQL 한 곳에 둔다 |
| 스풀 볼륨(mediamtx·api 공유)과 저장 볼륨(api만)은 이미지에 65532 소유 디렉터리를 만들어 named volume이 그 소유자로 초기화되게 한다 | 컨테이너는 non-root·read-only rootfs를 유지한다(check-hardening 통과) |

## Rejected alternatives

- HLS VOD로 패키징해 hls.js 재사용: fMP4 박스를 파싱해 byte range 플레이리스트를 만들어야 하고, 세그먼트 요청마다 서명이 필요하다. 일반 video로 충분했다.
- MediaMTX playback 서버(`/get`)로 재생: 파일이 MediaMTX 디스크에 남아야 해서 S3 저장소와 맞지 않는다.
- hook 요청 안에서 동기 업로드: 요청 10초 상한과 MediaMTX wget 대기에 걸린다.
- `rsa` crate에 deny ignore: 원격에서 서명 시간을 잴 수 있는 경로라 받아들이지 않았다.
- 삭제된 방송의 녹화 업로드를 claim에서 건너뛰기(리뷰 F12): 읽기 경로가 이미 막는다. 객체 정리는 보존 정책과 함께 후속으로 한다.
- 방송 상세 녹화 목록 페이지네이션(리뷰 F13): 50건은 30분 세그먼트로 25시간이다. 범위 밖.

## Cautions when touching this

- hook이 유실되면(API가 그 순간 내려가 있으면) 그 세그먼트는 등록되지 않고 `recordDeleteAfter: 7d`가 지운다. 스풀을 훑어 보정하는 경로는 없다.
- 스풀은 호스트 하나의 named volume이다. API를 다른 호스트로 늘리면 스풀을 공유하는 방법부터 바꿔야 한다. 로컬 서명 키도 인스턴스마다 다르다.
- CloudFront URL은 방송 삭제 뒤 최대 5분 더 동작하고, S3 객체는 지우지 않는다.
- `RECORDING_MEDIA_ORIGIN`은 CSP에 그대로 들어간다. `https://host` 하나만 넣는다.
- (unverified) 실제 S3 버킷, CloudFront 배포, IAM Role 자격증명. #2 배포의 후속이다.
- (unverified) 긴 재생 중 URL이 만료됐을 때 Chrome이 `error`를 내고 플레이어가 이어 가는지. e2e는 위조 URL로 같은 경로만 확인했다.
- (unverified) iOS Safari에서 fMP4(Opus) 일반 video 재생. #3에서 iOS 내장 HLS는 Opus 디코딩에 실패했다.
