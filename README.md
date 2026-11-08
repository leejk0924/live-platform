## Credits

| 이름 | 쓰는 곳 | 라이선스 |
|---|---|---|
| [argon2](https://github.com/RustCrypto/password-hashes) | 비밀번호 해시(argon2id) | MIT OR Apache-2.0 |
| [sha2](https://github.com/RustCrypto/hashes) | 세션 토큰의 SHA-256 저장 | MIT OR Apache-2.0 |
| [hmac](https://github.com/RustCrypto/MACs) | 로컬 녹화 파일 URL의 HMAC-SHA256 서명 | MIT OR Apache-2.0 |
| [base64ct](https://github.com/RustCrypto/formats) | 세션 토큰 base64url 인코딩 | Apache-2.0 OR MIT |
| [getrandom](https://github.com/rust-random/getrandom) | 세션 토큰 난수(OS RNG) | MIT OR Apache-2.0 |
| [ed25519-dalek](https://github.com/dalek-cryptography/curve25519-dalek) | 송출 JWT(EdDSA) 서명 | BSD-3-Clause |
| [zeroize](https://github.com/RustCrypto/utils) | 송출 서명 키 seed를 쓴 뒤 메모리에서 지우기 | Apache-2.0 OR MIT |
| [reqwest](https://github.com/seanmonstar/reqwest) | MediaMTX Control API 호출(송출 종료 kick) | MIT OR Apache-2.0 |
| [p256](https://github.com/RustCrypto/elliptic-curves) | CloudFront 녹화 URL의 ECDSA P-256(SHA-256) 서명 | Apache-2.0 OR MIT |
| [rusty-s3](https://github.com/paolobarbolini/rusty-s3) | 녹화 업로드용 S3 presigned PUT URL(SigV4) 서명 | BSD-2-Clause |
| [subtle](https://github.com/dalek-cryptography/subtle) | MediaMTX 공유 비밀값의 상수 시간 비교 | BSD-3-Clause |
| [governor](https://github.com/boinkor-net/governor) | 가입·로그인 속도 제한 | MIT |
| [time](https://github.com/time-rs/time) | 방송 생성 시각의 DB 변환(sqlx `time` 기능)과 RFC 3339 응답 형식 | MIT OR Apache-2.0 |
| [futures-util](https://github.com/rust-lang/futures-rs) | 댓글 SSE 스트림 구성(`stream::unfold`) | MIT OR Apache-2.0 |
| [utoipa](https://github.com/juhaku/utoipa) | 핸들러에서 OpenAPI 명세 생성(`/api/openapi.json`, `web/src/api/openapi.json`) | MIT OR Apache-2.0 |
| [openapi-typescript](https://github.com/openapi-ts/openapi-typescript) | `web/src/api/openapi.json`에서 TypeScript 타입(`web/src/api/schema.d.ts`) 생성 | MIT |
| [openapi-fetch](https://github.com/openapi-ts/openapi-typescript/tree/main/packages/openapi-fetch) | 생성된 타입으로 API를 부르는 웹 클라이언트 | MIT |
| [hls.js](https://github.com/video-dev/hls.js) | 웹 LL-HLS 재생(MSE) | Apache-2.0 |
| [SecLists](https://github.com/danielmiessler/SecLists) `Passwords/Common-Credentials/xato-net-10-million-passwords-1000000.txt` (커밋 `c205c36`) | 유출 비밀번호 목록. 10~128자 항목을 소문자로 중복 제거해 `crates/infrastructure/data/breached-passwords.txt`에 담았다(117,571개) | MIT |
| [gitleaks](https://github.com/gitleaks/gitleaks) | 커밋 전(`.githooks/pre-commit`)과 CI의 비밀값 검사 | MIT |
| [distroless](https://github.com/GoogleContainerTools/distroless) | api, caddy 런타임 베이스 이미지 | Apache-2.0 |
