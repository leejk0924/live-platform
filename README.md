## Credits

| 이름 | 쓰는 곳 | 라이선스 |
|---|---|---|
| [argon2](https://github.com/RustCrypto/password-hashes) | 비밀번호 해시(argon2id) | MIT OR Apache-2.0 |
| [sha2](https://github.com/RustCrypto/hashes) | 세션 토큰의 SHA-256 저장 | MIT OR Apache-2.0 |
| [base64ct](https://github.com/RustCrypto/formats) | 세션 토큰 base64url 인코딩 | Apache-2.0 OR MIT |
| [getrandom](https://github.com/rust-random/getrandom) | 세션 토큰 난수(OS RNG) | MIT OR Apache-2.0 |
| [governor](https://github.com/boinkor-net/governor) | 가입·로그인 속도 제한 | MIT |
| [SecLists](https://github.com/danielmiessler/SecLists) `Passwords/Common-Credentials/10k-most-common.txt` | 유출 비밀번호 목록. 10자 이상 항목만 `crates/infrastructure/data/breached-passwords.txt`에 담았다 | MIT |
