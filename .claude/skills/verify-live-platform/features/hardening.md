# Security hardening at the edge

Caddy serves every response with security headers and a strict CSP, enforces request timeouts and size limits, and redirects HTTP to HTTPS without holding connections. The containers run non-root on read-only filesystems with pinned images.

## Sub-features
- Headers: CSP (strict everywhere, with a Toss-specific CSP only on `/pay`), `X-Content-Type-Options`, `Referrer-Policy`, `Permissions-Policy`, `frame-ancestors 'none'`. HSTS is `max-age=0` locally and `31536000` in prod.
- Slowloris: a half-sent header is dropped after 10 s, and the `:80` redirect server is covered too. Bodies are limited to 64 KB.
- Containers: non-root, read-only rootfs, `cap_drop: ALL`, digest-pinned images. Caddy and the api run on distroless images with no shell.
- Audit log lines for `login_failed`, `forbidden` and `publisher_kicked`, with no secrets.

## How to get to it (user POV)
Not a page. Every response from `https://localhost` carries these properties.

## Driving it with the shell check
```bash
bash deploy/check-hardening.sh local
```
It runs every check, prints each failure, and ends with one `ok: headers, /pay CSP, slowloris cutoff (…s), non-root and read-only rootfs …` line when all pass. The exit code is the number of failures. For browser-side proof, every Playwright journey collects CSP violations per browser context across navigations and ends with `noCspViolations` (lib.mjs), which fails on any.

Side effect: after a wrong-password login, `docker compose logs api | grep login_failed` shows the event, and the password never appears.

End state that proves it: `check-hardening.sh local` exits 0, and the journeys show zero CSP violations.

## Gotchas
- Production HSTS and ACME only show up on the real domain (`deploy/check-hardening.sh prod`, #2). Locally they stay unverified.
- A new page or third-party script needs its CSP entry scoped to the route that uses it, as `/pay` does. Never loosen the global policy to make a journey pass.
