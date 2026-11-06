# web

live-platform SPA (Vite + React + TypeScript). Node 24 (`.nvmrc`).

```bash
npm ci
npm run dev        # http://localhost:5173, proxies /api to http://localhost:8080
npm run lint
npm run typecheck
npm run build      # outputs dist/
```

## api를 같이 띄우기

`npm run dev`는 `/api`를 `http://localhost:8080`으로 넘긴다. 브라우저는 `Origin: http://localhost:5173`을 보내므로 api가 그 origin을 허용해야 쓰기 요청이 403이 되지 않는다. 저장소 루트에서 실행한다.

```bash
docker run -d --name lp-test-pg -e POSTGRES_PASSWORD=test -p 127.0.0.1:55432:5432 postgres:17.11-trixie

ALLOWED_ORIGINS=http://localhost:5173 \
DATABASE_URL=postgres://postgres:test@localhost:55432/postgres \
DEMO_LOGIN_ENABLED=true \
cargo run -p api
```

- 같은 Postgres를 `cargo test --workspace -- --include-ignored`의 `DATABASE_URL`로도 쓴다. api는 시작할 때 마이그레이션을 적용한다.
- `DEMO_LOGIN_ENABLED`는 선택이다. 켜면 데모 계정을 만들고 데모 로그인 버튼이 동작한다.
- 세션 쿠키는 `__Host-` 접두사라 `Secure`가 붙는다. 브라우저는 `http://localhost`를 안전한 출처로 보므로 그대로 동작한다.
