#!/usr/bin/env bash
# Checks the hardening a running stack must show from outside and from docker.
# Usage: deploy/check-hardening.sh local|prod
#   local  compose.yml only; expects HSTS max-age=0 (the internal CA is accepted by hand)
#   prod   compose.yml + compose.prod.yml; expects a one-year HSTS
set -euo pipefail

mode="${1:-}"
case "$mode" in
    local) files=(-f compose.yml); hsts='max-age=0'; curl_tls=(--insecure) ;;
    prod) files=(-f compose.yml -f compose.prod.yml); hsts='max-age=31536000'; curl_tls=() ;;
    *) echo "usage: deploy/check-hardening.sh local|prod" >&2; exit 2 ;;
esac

cd "$(dirname "$0")/.."
host="$(docker compose "${files[@]}" exec -T caddy caddy environ | sed -n 's/^SITE_ADDRESS=//p' | tr -d '\r')"
failed=0
fail() { echo "FAIL: $*" >&2; failed=1; }

headers="$(curl -fsS "${curl_tls[@]}" -D - -o /dev/null "https://$host/" | tr -d '\r')"
expect_header() {
    grep -i "^$1:" <<<"$headers" | grep -qF "$2" || fail "header $1 lacks '$2'"
}
expect_header Content-Security-Policy "script-src 'self'"
expect_header Content-Security-Policy "frame-ancestors 'none'"
expect_header Strict-Transport-Security "$hsts"
expect_header X-Content-Type-Options nosniff
expect_header Referrer-Policy strict-origin-when-cross-origin
expect_header Permissions-Policy "camera=(self), microphone=(self), display-capture=(self)"
grep -qi '^server:' <<<"$headers" && fail "Server header is exposed"

# Slowloris: headers that never finish must be cut off by Caddy's 10s read_header timeout.
# Plain HTTP on :80 is enough, since the timeouts are global options that every listener shares.
start=$SECONDS
exec 3<>"/dev/tcp/$host/80"
printf 'GET / HTTP/1.1\r\nHost: %s\r\n' "$host" >&3
while read -r -t 30 _ <&3; do :; done || true
exec 3<&-
elapsed=$((SECONDS - start))
[ "$elapsed" -lt 20 ] || fail "an unfinished request header stayed open for ${elapsed}s"

for service in caddy api mediamtx postgres; do
    id="$(docker compose "${files[@]}" ps -q "$service")"
    read -r user readonly < <(docker inspect --format '{{.Config.User}} {{.HostConfig.ReadonlyRootfs}}' "$id")
    case "$user" in ''|0|0:*|root|root:*) fail "$service runs as root" ;; esac
    [ "$readonly" = true ] || fail "$service rootfs is writable"
done

[ "$failed" = 0 ] && echo "ok: headers, slowloris cutoff (${elapsed}s), non-root and read-only rootfs on https://$host"
exit "$failed"
