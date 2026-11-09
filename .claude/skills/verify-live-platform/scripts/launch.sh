#!/usr/bin/env bash
# Build and start the compose stack from the current checkout, then record what was launched.
# Usage: bash .claude/skills/verify-live-platform/scripts/launch.sh
# Ready = deploy.sh printed "ok: https://localhost is up" (all healthchecks passed and /api/healthz said ok through Caddy).
set -euo pipefail
. "$(dirname "$0")/env.sh"
cd "$REPO"

[ -f .env ] || { echo ".env missing: cp .env.example .env and fill PUBLISH_JWT_PRIVATE_KEY, MEDIA_HOOK_SECRET (see .env.example)" >&2; exit 1; }

# Remember whether the stack was already up, so cleanup only stops a stack this skill started.
running="$(docker compose ps --status running -q 2>/dev/null | wc -l | tr -d ' ')"
started_by_us=no
[ "$running" = 0 ] && started_by_us=yes

bash deploy/deploy.sh local

dirty=no
[ -n "$(git status --porcelain --untracked-files=no)" ] && dirty=yes
{
    echo "commit=$(git rev-parse HEAD)"
    echo "dirty=$dirty"
    echo "started_by_us=$started_by_us"
    echo "launched_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
} > "$STATE"
echo "ok: launched $(git rev-parse --short HEAD) (dirty=$dirty, started_by_us=$started_by_us)"
