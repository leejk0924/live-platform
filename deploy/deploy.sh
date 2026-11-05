#!/usr/bin/env bash
# Brings the current checkout up with docker compose and checks the health endpoint through Caddy.
# Usage: deploy/deploy.sh local|prod
#   local  compose.yml only, https://localhost with Caddy's internal CA
#   prod   compose.yml + compose.prod.yml, https://$DOMAIN with Let's Encrypt
# It deploys whatever is checked out; update with git first (see deploy/README.md).
set -euo pipefail

mode="${1:-}"
case "$mode" in
    local) files=(-f compose.yml) ;;
    prod) files=(-f compose.yml -f compose.prod.yml) ;;
    *) echo "usage: deploy/deploy.sh local|prod" >&2; exit 2 ;;
esac

cd "$(dirname "$0")/.."

[ -f .env ] || { echo ".env not found: copy .env.example to .env and fill it" >&2; exit 1; }
if [ "$mode" = prod ] && grep -q 'change-me' .env; then
    echo ".env still has change-me placeholders" >&2
    exit 1
fi

docker compose "${files[@]}" up -d --build --wait --remove-orphans

if [ "$mode" = prod ]; then
    host="$(sed -n 's/^DOMAIN=//p' .env)"
    insecure=()
else
    host=localhost
    # The local certificate comes from Caddy's internal CA, which this machine does not trust.
    insecure=(--insecure)
fi

body="$(curl -fsS --max-time 10 "${insecure[@]}" "https://$host/api/healthz")"
[ "$body" = ok ] || { echo "healthz answered '$body', expected 'ok'" >&2; exit 1; }
echo "ok: https://$host is up ($(git rev-parse --short HEAD))"
